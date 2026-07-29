//! A live HTTP server for the dashboard — the #59/#65 "real dashboard
//! parity" pivot: an axum backend exposing indexed-repo data as JSON,
//! plus static-asset serving for a WASM frontend (`repowise-web`),
//! replacing the one-shot `repowise dashboard` static HTML page with a
//! long-running server an SPA can poll/query live.
//!
//! Phase 0 proved the architecture with `GET /api/overview` alone.
//! Phase 1 added the rest of the static dashboard's views onto the same
//! JSON-API shape: `/api/health`, `/api/hotspots`, `/api/decisions`,
//! `/api/symbols`. Phase 2 added `/api/wiki-pages` and `/api/wiki`
//! (wiki-page drill-down, matching the static dashboard's file-path
//! links) and `/api/search` (instant search over files and symbols).
//! Phase 3 added `/api/graph`, a file-level import dependency graph for
//! a visual graph view. Phase 4 added `/api/ownership` (per-file
//! git-blame breakdown), an optional `?file=` filter on `/api/decisions`
//! (decisions linked to one file, for a per-file decision tracker), and
//! `/api/dead-code` (confidence-tiered dead-code candidates). Phase 5
//! added `POST /api/chat`, the last static-parity view: a chat endpoint
//! over `repowise-llm`. `{"available": false}` when
//! `REPOWISE_LLM_BASE_URL` isn't set, same opt-in convention every other
//! LLM feature in this port uses.
//!
//! Retrieval itself moved to `repowise_llm::retrieval` (PR #306) so this
//! endpoint and the `get_answer` MCP tool share one implementation and
//! can't drift. It embeds the question against `repowise-llm`'s
//! persisted embedding index, embedding only the files that index
//! doesn't already cover (#308) -- falling back to keyword-substring
//! matching if the embeddings call itself fails (e.g. an endpoint that
//! doesn't implement `/v1/embeddings`), so a chat reply is never blocked
//! by that.
//!
//! `/api/search` stays substring-only (no embeddings there -- an API
//! call per keystroke would make instant search not instant) but is
//! now PageRank-biased, #63's second slice and the cheaper alternative
//! its own open questions suggested: matched files/symbols are ranked
//! by `repowise-graph`'s already-computed `dependents_of`/
//! `call_in_degree` rather than plain alphabetical, so a
//! heavily-depended-on file or heavily-called symbol surfaces above an
//! equally-matching but less-connected one, no new analysis needed.
//!
//! Issue #65 (the live-server-dependent dashboard features, split out
//! after the #59/#65 pivot phases) also tracks a live job banner. `GET
//! /api/reindex`/`POST /api/reindex` (this module now) add that: `POST`
//! kicks off a background reindex (`repowise_parser::build_index`,
//! shared with `repowise-cli`'s own `init`/`update` commands so there's
//! exactly one indexing implementation) unless one's already running;
//! `GET` reports the current job status (idle/running/completed/
//! failed) for the dashboard to poll. `GET /api/settings` is a
//! read-only Settings view: repo root, indexed file counts, whether
//! git history/wiki pages are available, and whether an LLM is
//! configured (and which model) -- this port has no persisted
//! repo-level exclusion/generation config or global server/webhook/MCP
//! config to expose a write endpoint for, so surfacing what the server
//! already knows about itself is this slice's honest scope. `GET
//! /api/usage` is #65's cost-tracking view -- the last of its five
//! bundled features -- reporting running totals (chat-call count,
//! prompt/completion/total tokens) tallied from every `/api/chat` call
//! whose response reported `usage`
//! (`repowise_llm::complete_messages_with_usage`). In-memory for this
//! server process only (reset on restart, not a persisted history
//! across sessions) and token counts, not a dollar cost -- this port
//! has no per-model pricing table.
//!
//! `GET /api/workspace-repos` is the first slice of issue #64 (multi-
//! repo/workspace support): when this server was started with
//! `--workspace <path>` (a `repowise-workspace` TOML file naming member
//! repos), it reports every configured repo's name/path/indexed status
//! (`{"available": true, "repos": [...]}`), or `{"available": false}`
//! when no `--workspace` was given -- same degrade-gracefully shape as
//! every other optional-data endpoint here. This is deliberately just a
//! listing: no cross-repo dependency resolution exists anywhere in this
//! port, so `get_architecture`/`get_blast_radius` and the dashboard's
//! system-map/conformance/contracts/co-changes views (also #64) are
//! left for a follow-up, and there's no way to switch which repo the
//! rest of this server's endpoints operate on -- `root` stays fixed for
//! the life of the process, same as before this endpoint existed.
//!
//! Requires a prior `repowise init`/`update`, same as every other
//! command that reads `.repowise/index.json`.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use repowise_core::RepoIndex;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tower_http::services::ServeDir;

/// `file`'s path relative to `root`, for JSON responses -- callers (a
/// browser-side SPA) have no business seeing this host's absolute
/// filesystem layout.
fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

#[derive(Clone)]
struct AppState {
    root: Arc<PathBuf>,
    /// Resolved once at server startup (`LlmConfig::from_env()`), not
    /// re-read per request -- keeps `post_chat` a pure function of its
    /// state, and lets tests inject a fixture config directly instead of
    /// mutating process env vars (which would race across parallel
    /// tests).
    llm_config: Arc<Option<repowise_llm::LlmConfig>>,
    /// Resolved once at server startup from `--workspace <path>`
    /// (`None` if that flag was omitted, `Some(vec![])` for a
    /// workspace file naming zero repos) -- see this module's own
    /// module doc comment for the #64 workspace-listing feature this
    /// backs.
    workspace_repos: Arc<Option<Vec<repowise_workspace::ResolvedWorkspaceRepo>>>,
    reindex_job: ReindexJob,
    usage: UsageTracker,
}

/// Running token-usage totals for this server process, tallied across
/// every `/api/chat` call that got a usage-reporting response back --
/// in-memory and reset on restart, not a persisted history across
/// sessions (that needs its own design pass, same as every other
/// "needs persistence" gap noted elsewhere in this module). Token
/// counts, not a dollar cost: see `repowise_llm::Usage`'s own doc
/// comment for why.
#[derive(Serialize, Clone, Copy, Default)]
struct UsageTotalsDto {
    chat_call_count: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Clone, Default)]
struct UsageTracker(Arc<Mutex<UsageTotalsDto>>);

impl UsageTracker {
    fn new() -> Self {
        UsageTracker::default()
    }

    fn record(&self, usage: repowise_llm::Usage) {
        let mut totals = self.0.lock().unwrap();
        totals.chat_call_count += 1;
        totals.prompt_tokens += usage.prompt_tokens;
        totals.completion_tokens += usage.completion_tokens;
        totals.total_tokens += usage.total_tokens;
    }

    fn snapshot(&self) -> UsageTotalsDto {
        *self.0.lock().unwrap()
    }
}

/// The live job banner's status shape, polled by the dashboard via `GET
/// /api/reindex`. Internally tagged so the wire format is a flat
/// `{"status": "completed", "file_count": ..., ...}` object rather than a
/// separate boolean/variant-name split.
#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ReindexStatusDto {
    Idle,
    Running,
    Completed {
        file_count: usize,
        other_file_count: usize,
        duration_ms: u64,
    },
    Failed {
        error: String,
    },
}

/// Shared, atomically-updated background-job state for `/api/reindex`.
/// `try_start` is the only way to transition into `Running`, and refuses
/// to do so if a job is already running -- the one concurrency guard the
/// whole feature needs, since there's no job queue, just "at most one
/// reindex in flight".
#[derive(Clone)]
struct ReindexJob(Arc<Mutex<ReindexStatusDto>>);

impl ReindexJob {
    fn new() -> Self {
        ReindexJob(Arc::new(Mutex::new(ReindexStatusDto::Idle)))
    }

    fn snapshot(&self) -> ReindexStatusDto {
        self.0.lock().unwrap().clone()
    }

    /// Returns `true` and transitions to `Running` if no job is
    /// currently in flight; returns `false` (and leaves the state alone)
    /// if one already is.
    fn try_start(&self) -> bool {
        let mut guard = self.0.lock().unwrap();
        if matches!(*guard, ReindexStatusDto::Running) {
            false
        } else {
            *guard = ReindexStatusDto::Running;
            true
        }
    }

    fn finish(&self, result: ReindexStatusDto) {
        *self.0.lock().unwrap() = result;
    }
}

/// A JSON-serializable copy of `repowise_graph::Overview` — kept as a
/// separate DTO here rather than adding `Serialize` directly onto
/// `Overview` itself, since that type has no other reason to carry a
/// JSON-wire-format dependency; `repowise-server` is the one crate
/// that needs one.
#[derive(Serialize)]
struct OverviewDto {
    file_count: usize,
    other_file_count: usize,
    by_language: Vec<(String, usize)>,
    symbol_counts: Vec<(String, usize)>,
    total_lines: usize,
    import_edges: usize,
    call_edges: usize,
    unresolved_imports: usize,
    unresolved_calls: usize,
    most_depended_on: Vec<(String, usize)>,
}

impl OverviewDto {
    fn from_overview(root: &Path, o: &repowise_graph::Overview) -> Self {
        OverviewDto {
            file_count: o.file_count,
            other_file_count: o.other_file_count,
            by_language: o.by_language.clone(),
            symbol_counts: o.symbol_counts.clone(),
            total_lines: o.total_lines,
            import_edges: o.import_edges,
            call_edges: o.call_edges,
            unresolved_imports: o.unresolved_imports,
            unresolved_calls: o.unresolved_calls,
            most_depended_on: o
                .most_depended_on
                .iter()
                .map(|(path, count)| (relative(root, path), *count))
                .collect(),
        }
    }
}

/// A JSON-serializable summary of a `repowise_health::HealthReport`:
/// the same numbers and "lowest-scoring files" slice the static
/// dashboard's health section renders, not the full per-finding detail.
#[derive(Serialize)]
struct HealthDto {
    average_score: f64,
    file_count: usize,
    finding_count: usize,
    by_kind: Vec<FindingKindCountDto>,
    worst_files: Vec<FileHealthDto>,
}

#[derive(Serialize)]
struct FindingKindCountDto {
    kind: String,
    count: usize,
}

#[derive(Serialize)]
struct FileHealthDto {
    file: String,
    score: f64,
    finding_count: usize,
}

/// How many of the worst-scoring files to include — matches the static
/// dashboard's own `take(15)`.
const WORST_FILES_LIMIT: usize = 15;
const HOTSPOTS_LIMIT: usize = 15;

/// A JSON-serializable `repowise_git::Hotspot`. `available: false` (with
/// an empty list) means this root has no git history to analyze --
/// distinct from "available, but no file has both history and
/// complexity", which is `available: true` with an empty list.
#[derive(Serialize)]
struct HotspotsDto {
    available: bool,
    hotspots: Vec<HotspotDto>,
}

#[derive(Serialize)]
struct HotspotDto {
    file: String,
    churn: usize,
    total_complexity: usize,
    bugfix_commits: usize,
    score: usize,
    decayed_score: f64,
}

#[derive(Serialize)]
struct DecisionDto {
    id: String,
    title: String,
    status: Option<String>,
    superseded_by: Option<String>,
    linked_file_count: usize,
    /// Where this came from, e.g. `adr:docs/adr/0001.md` or
    /// `inferred:src/queue.rs:12 by smart`.
    source: String,
    /// True when a **model inferred** this decision from code rather
    /// than reading it from something a person wrote. Rendered as a
    /// visible badge, not just carried in the payload -- a reader
    /// weighing "we chose X because Y" needs to know which kind it is.
    inferred: bool,
}

/// `GET /api/decisions`.
///
/// An object rather than a bare array so it can carry
/// `inferred_source`. An empty list is ambiguous between "this repo has
/// no inferred decisions" and "the pass that infers them was never
/// run", and only one of those is a fact about the codebase.
#[derive(Serialize)]
struct DecisionsDto {
    decisions: Vec<DecisionDto>,
    inferred_source: String,
}

/// One-line label for a decision's origin, shared by both decision
/// endpoints so they can't describe the same record differently.
fn source_label(root: &Path, source: &repowise_adr::DecisionSource) -> String {
    use repowise_adr::DecisionSource as S;
    match source {
        S::Adr { file } => format!("adr:{}", relative(root, file)),
        S::CommitMessage { hash, author } => {
            format!("commit:{} by {author}", &hash[..hash.len().min(7)])
        }
        S::PullRequest { number, author } => format!("pr:{number} by {author}"),
        S::CodeComment { file, line } => format!("comment:{}:{line}", relative(root, file)),
        S::InlineMarker { file, line, marker } => {
            format!("marker:{marker}:{}:{line}", relative(root, file))
        }
        S::Changelog { file, section } => format!("changelog:{section}:{}", relative(root, file)),
        S::Inferred { file, line, model } => {
            format!("inferred:{}:{line} by {model}", relative(root, file))
        }
    }
}

#[derive(Serialize, Clone)]
struct SymbolDto {
    name: String,
    kind: String,
    file: String,
    start_line: usize,
}

/// Every indexed file's path relative to `root`, restricted to those
/// with a `repowise-docs` wiki page already on disk -- the same
/// "check disk, don't generate" convention the static dashboard uses.
fn wiki_indexed_files(root: &Path, index: &RepoIndex) -> Vec<(String, PathBuf)> {
    index
        .files
        .iter()
        .map(|f| (relative(root, &f.path), f.path.clone()))
        .filter(|(_, path)| repowise_docs::wiki_page_path(root, path).is_file())
        .collect()
}

#[derive(Deserialize)]
struct WikiQuery {
    path: String,
}

#[derive(Serialize)]
struct WikiDto {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Serialize)]
struct SearchDto {
    files: Vec<String>,
    symbols: Vec<SymbolDto>,
}

/// How many matches `/api/search` returns per category -- an instant
/// search box needs a short, glanceable list, not the whole index.
const SEARCH_LIMIT: usize = 20;

#[derive(Serialize)]
struct GraphNodeDto {
    id: String,
    language: String,
}

#[derive(Serialize)]
struct GraphEdgeDto {
    from: String,
    to: String,
}

#[derive(Serialize)]
struct GraphDto {
    nodes: Vec<GraphNodeDto>,
    edges: Vec<GraphEdgeDto>,
    /// `true` when this root has more files than `GRAPH_NODE_LIMIT` and
    /// the graph below was cut down to the most-connected ones -- the
    /// frontend surfaces this rather than silently rendering a partial
    /// graph that looks complete.
    truncated: bool,
}

/// A force-directed SVG layout of the whole file-import graph gets
/// unreadable (and the client-side layout expensive) well before most
/// real repos' file counts; keep the view to the most-connected files,
/// which is also the most useful part of the graph to look at.
const GRAPH_NODE_LIMIT: usize = 150;

#[derive(Deserialize)]
struct OwnershipQuery {
    path: String,
}

#[derive(Serialize)]
struct OwnershipEntryDto {
    author: String,
    lines: usize,
    percentage: f64,
}

/// `available: false` covers both "not a git repo" and "path doesn't
/// match an indexed file" -- `git blame` failures degrade gracefully
/// the same way `/api/hotspots` does, rather than a 500.
#[derive(Serialize)]
struct OwnershipDto {
    available: bool,
    owners: Vec<OwnershipEntryDto>,
}

#[derive(Deserialize)]
struct DeadCodeQuery {
    #[serde(default)]
    min_confidence: Option<String>,
}

#[derive(Serialize)]
struct DeadCodeCandidateDto {
    file: String,
    symbol: String,
    line: usize,
    confidence: String,
    risk_factors: Vec<String>,
}

#[derive(Serialize)]
struct DeadCodeDto {
    candidates: Vec<DeadCodeCandidateDto>,
    /// How many candidates matched `min_confidence` before truncation to
    /// `DEAD_CODE_LIMIT` -- mirrors the `get_dead_code` MCP tool's own
    /// `total_matching` field, for the same "don't silently truncate"
    /// reason.
    total_matching: usize,
}

/// Matches the `get_dead_code` MCP tool's own default `limit`.
const DEAD_CODE_LIMIT: usize = 50;

/// Symbol detail, for `GET /api/symbol` (issue #263).
#[derive(Serialize)]
struct SymbolDetailDto {
    found: bool,
    name: String,
    kind: String,
    file: String,
    start_line: usize,
    end_line: usize,
    parent: Option<String>,
    complexity: usize,
    max_nesting_depth: usize,
    /// Symbols this one calls, and symbols that call it -- both limited
    /// to what this port's heuristic resolution could actually resolve.
    callees: Vec<String>,
    callers: Vec<String>,
    /// Unresolved calls out of this symbol. Reported so an empty
    /// `callees` list isn't read as "this calls nothing".
    unresolved_callee_count: usize,
}

/// Decision detail, for `GET /api/decision` (issue #263).
#[derive(Serialize)]
struct DecisionDetailDto {
    found: bool,
    id: String,
    title: String,
    status: Option<String>,
    /// Set when a later decision supersedes this one. Displaying a
    /// superseded decision without saying so would be actively
    /// misleading -- it reads as current guidance.
    superseded_by: Option<String>,
    /// The decision this one supersedes, if any -- the other direction
    /// of the same lineage, resolved by scanning the full set.
    supersedes: Option<String>,
    body: String,
    linked_files: Vec<String>,
    /// Where this came from. Empty when `found` is false.
    source: String,
    /// True when a model inferred this from code rather than reading it
    /// from something a person wrote. The detail view is where someone
    /// goes to decide whether to act on a decision, so it is exactly
    /// where this must not be missing.
    inferred: bool,
}

/// Commit-activity stats, for `GET /api/stats` (issue #262).
#[derive(Serialize)]
struct StatsDto {
    available: bool,
    /// True when the clone is shallow. Surfaced because truncated
    /// history doesn't make a trend chart *fail*, it makes it quietly
    /// under-report -- which is exactly where a chart misleads.
    shallow: bool,
    commit_count: usize,
    /// `[day][hour]`, day 0 = Sunday, hour 0 = midnight, both UTC.
    punch_card: Vec<Vec<usize>>,
    /// Commits per week, oldest first; the last entry is the current week.
    weekly_trend: Vec<usize>,
    /// Named explicitly so the client can state it. A punch card whose
    /// timezone is implied is a punch card that can be misread.
    timezone: &'static str,
}

/// One indexed file, for `GET /api/files` (issue #261).
#[derive(Serialize)]
struct FileEntryDto {
    path: String,
    language: String,
    lines: usize,
    /// `None` when health scoring produced no entry for this file.
    /// Deliberately optional rather than defaulted to 10.0: a file with
    /// no score is not a healthy file, and the treemap needs to render
    /// those distinctly instead of coloring them "good".
    score: Option<f64>,
    finding_count: usize,
}

#[derive(Serialize)]
struct FilesDto {
    files: Vec<FileEntryDto>,
    total_lines: usize,
    /// False when health scoring was unavailable entirely, so the client
    /// can degrade to an uncolored list rather than implying every file
    /// is unscored for its own reason.
    health_available: bool,
}

/// How many files `GET /api/contributors` blames before stopping.
///
/// `ownership_of` shells out to `git blame --line-porcelain` **once per
/// file**, so an unbounded sweep is one subprocess per indexed file --
/// fine at 85 files, not fine at several thousand, and a dashboard
/// endpoint that takes 30s is not usable.
///
/// Bounding rather than caching is deliberate: a cache would need an
/// invalidation story (the index has one, git history doesn't), whereas
/// a bound is stateless and its cost is knowable up front. Files are
/// taken largest-first, since they carry most of the repo's lines and so
/// dominate any ownership share; the response reports how many were
/// sampled so the UI can say so rather than implying a full sweep.
const CONTRIBUTORS_FILE_LIMIT: usize = 200;

#[derive(Serialize)]
struct ContributorDto {
    author: String,
    lines_owned: usize,
    /// Share of all sampled lines, `0.0..=100.0`.
    percent: f64,
    /// Files where this author owns at least one line.
    files_touched: usize,
}

#[derive(Serialize)]
struct ContributorsDto {
    available: bool,
    contributors: Vec<ContributorDto>,
    /// How many sampled files have each bus factor, as `(bus_factor,
    /// file_count)` ascending. Bus factor 1 first, since that's the
    /// concentration risk worth seeing.
    bus_factor_distribution: Vec<(usize, usize)>,
    files_sampled: usize,
    files_total: usize,
    /// True only when `CONTRIBUTORS_FILE_LIMIT` actually truncated the
    /// sweep.
    ///
    /// Kept separate from `files_sampled < files_total` because those
    /// are different facts: files are also skipped when they cannot be
    /// blamed at all (untracked, or never committed). Conflating them
    /// would report "bounded sample" on a repo where the bound never
    /// applied -- which is what a first cut of this endpoint did.
    limit_applied: bool,
    /// Indexed files that could not be blamed, and so contributed
    /// nothing.
    files_unblameable: usize,
}

/// Per-file coverage, for `GET /api/coverage` (issue #257).
#[derive(Serialize)]
struct FileCoverageDto {
    path: String,
    /// Percentage of this file's *known* lines that executed at least
    /// once. Only present for measured files -- see `CoverageDto`.
    percent: f64,
    lines_known: usize,
    lines_hit: usize,
}

#[derive(Serialize)]
struct CoverageDto {
    /// `false` when no coverage has been ingested. Mirrors
    /// `/api/ownership`'s flag: a missing-data state is reported, not
    /// raised as an error.
    available: bool,
    /// Files that appear in an ingested report, least-covered first.
    ///
    /// **Only measured files appear here.** A file no report mentioned
    /// is absent rather than listed at 0% -- `CoverageData::line_coverage_of`
    /// returns `None` vs `Some(0.0)` precisely to keep "never measured"
    /// and "measured, nothing ran" apart, and flattening them into one
    /// list would undo that distinction at the API boundary.
    files: Vec<FileCoverageDto>,
    /// Indexed files with no coverage record at all. Reported as a count
    /// so the UI can say "N files unmeasured" rather than implying the
    /// measured set is the whole repo.
    unmeasured_files: usize,
    mean_percent: f64,
    /// Whether the ingested reports carried per-test contexts. Without
    /// it `repowise impacted-tests` cannot run, which is worth showing.
    has_per_test_map: bool,
    test_contexts: usize,
}

#[derive(Deserialize)]
struct ChatTurnDto {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatRequestDto {
    /// Full conversation so far, oldest first, ending with the new user
    /// turn -- this endpoint is otherwise stateless, so the frontend
    /// owns history and resends it every call.
    history: Vec<ChatTurnDto>,
}

#[derive(Serialize)]
struct ChatResponseDto {
    /// `false` when `REPOWISE_LLM_BASE_URL` isn't set -- the same
    /// opt-in convention every other LLM feature in this port uses.
    /// `reply` is `None` in that case.
    available: bool,
    reply: Option<String>,
    /// Repo-relative files the answer drew on. Empty means the answer
    /// came from no sources -- worth showing, since an ungrounded answer
    /// and a well-sourced one look identical otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cited: Vec<String>,
    /// `semantic` or `keyword`. The latter is the degraded fallback.
    #[serde(default)]
    retrieval_mode: String,
    /// Present only when retrieval degraded, explaining what that means
    /// for the answer above it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retrieval_caveat: Option<String>,
    /// Files whose vectors were reused from the persisted embedding
    /// index vs. embedded fresh for this call. `semantic` mode only --
    /// coverage is always complete either way, so this is shown as a
    /// performance fact rather than a caveat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vectors_reused: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vectors_embedded_now: Option<usize>,
}

/// A read-only snapshot of this server's current configuration and
/// indexed-repo status -- the Settings view. No write endpoint exists:
/// this port has no persisted repo-level exclusion/generation config or
/// global server/webhook/MCP config to write to yet, so surfacing what
/// the server already knows about itself is this slice's honest scope.
#[derive(Serialize)]
struct SettingsDto {
    root: String,
    file_count: usize,
    other_file_count: usize,
    git_available: bool,
    wiki_pages_available: bool,
    llm_configured: bool,
    llm_model: Option<String>,
}

struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(err: E) -> Self {
        ApiError(err.into())
    }
}

async fn get_overview(State(state): State<AppState>) -> Result<Json<OverviewDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let graph = repowise_graph::RepoGraph::build(&index);
    let overview = graph.overview(&index);
    Ok(Json(OverviewDto::from_overview(&state.root, &overview)))
}

async fn get_health(State(state): State<AppState>) -> Result<Json<HealthDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let graph = repowise_graph::RepoGraph::build(&index);
    let health = repowise_health::analyze(&index, &graph);

    let by_kind = health
        .findings_by_kind()
        .into_iter()
        .map(|(kind, count)| FindingKindCountDto {
            kind: kind.label().to_string(),
            count,
        })
        .collect();

    let worst_files = health
        .file_scores
        .iter()
        .filter(|f| f.finding_count > 0)
        .take(WORST_FILES_LIMIT)
        .map(|f| FileHealthDto {
            file: relative(&state.root, &f.file),
            score: f.score,
            finding_count: f.finding_count,
        })
        .collect();

    Ok(Json(HealthDto {
        average_score: health.average_score,
        file_count: health.file_scores.len(),
        finding_count: health.findings.len(),
        by_kind,
        worst_files,
    }))
}

async fn get_hotspots(State(state): State<AppState>) -> Result<Json<HotspotsDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let dto = match repowise_git::GitAnalytics::collect(&state.root) {
        Ok(analytics) => {
            let hotspots = repowise_git::hotspots(&index, &analytics);
            HotspotsDto {
                available: true,
                hotspots: hotspots
                    .iter()
                    .take(HOTSPOTS_LIMIT)
                    .map(|h| HotspotDto {
                        file: relative(&state.root, &h.file),
                        churn: h.churn,
                        total_complexity: h.total_complexity,
                        bugfix_commits: h.bugfix_commits,
                        score: h.score,
                        decayed_score: h.decayed_score,
                    })
                    .collect(),
            }
        }
        Err(_) => HotspotsDto {
            available: false,
            hotspots: Vec::new(),
        },
    };
    Ok(Json(dto))
}

#[derive(Deserialize)]
struct SymbolDetailQuery {
    file: String,
    line: usize,
}

#[derive(Deserialize)]
struct DecisionDetailQuery {
    id: String,
}

#[derive(Deserialize)]
struct DecisionsQuery {
    /// Optional relative file path -- when given, only decisions linked
    /// to that file are returned. Powers the per-file decision-tracker
    /// panel; omitted entirely, this endpoint behaves exactly as it did
    /// before (every mined decision), which the repo-wide "Architectural
    /// decisions" section still relies on.
    #[serde(default)]
    file: Option<String>,
}

async fn get_decisions(
    State(state): State<AppState>,
    Query(query): Query<DecisionsQuery>,
) -> Result<Json<DecisionsDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let (decisions, inferred_state) = repowise_adr::mine_reporting(&index)
        .unwrap_or_else(|_| (Vec::new(), repowise_adr::InferredState::NotGenerated));
    Ok(Json(DecisionsDto {
        decisions: decisions
            .into_iter()
            .filter(|d| match &query.file {
                None => true,
                Some(rel) => d
                    .linked_files
                    .iter()
                    .any(|f| relative(&state.root, f) == *rel),
            })
            .map(|d| DecisionDto {
                source: source_label(&state.root, &d.source),
                inferred: d.source.is_inferred(),
                id: d.id,
                title: d.title,
                status: d.status,
                superseded_by: d.superseded_by,
                linked_file_count: d.linked_files.len(),
            })
            .collect(),
        inferred_source: inferred_state.describe(),
    }))
}

async fn get_symbols(State(state): State<AppState>) -> Result<Json<Vec<SymbolDto>>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let mut symbols: Vec<SymbolDto> = index
        .files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .map(|s| SymbolDto {
            name: s.name.clone(),
            kind: s.kind.label().to_string(),
            file: relative(&state.root, &s.file),
            start_line: s.start_line,
        })
        .collect();
    symbols.sort_by(|a, b| a.file.cmp(&b.file).then(a.start_line.cmp(&b.start_line)));
    Ok(Json(symbols))
}

async fn get_wiki_pages(State(state): State<AppState>) -> Result<Json<Vec<String>>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let mut pages: Vec<String> = wiki_indexed_files(&state.root, &index)
        .into_iter()
        .map(|(rel, _)| rel)
        .collect();
    pages.sort();
    Ok(Json(pages))
}

/// Serves the raw markdown of a single indexed file's wiki page.
/// `path` is matched against the exact set of indexed-and-has-a-wiki-page
/// relative paths (the same set `/api/wiki-pages` returns) rather than
/// joined onto `root` directly, so an arbitrary `path` query value can't
/// escape `.repowise/wiki/` via `..` segments.
async fn get_wiki(
    State(state): State<AppState>,
    Query(query): Query<WikiQuery>,
) -> Result<Response, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let found = wiki_indexed_files(&state.root, &index)
        .into_iter()
        .find(|(rel, _)| *rel == query.path);
    let Some((rel, file)) = found else {
        return Ok((StatusCode::NOT_FOUND, "no wiki page for that path").into_response());
    };
    let wiki_path = repowise_docs::wiki_page_path(&state.root, &file);
    let content = std::fs::read_to_string(&wiki_path)?;
    Ok(Json(WikiDto { path: rel, content }).into_response())
}

async fn get_search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let needle = query.q.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Json(SearchDto {
            files: Vec::new(),
            symbols: Vec::new(),
        }));
    }

    // PageRank-biased ranking (issue #63's cheaper-than-embeddings
    // intermediate step): among substring matches, files/symbols with
    // more dependents/callers -- already computed by `repowise-graph`,
    // no new analysis needed -- rank above equally-matching but less-
    // depended-on ones. No network call, so instant search stays
    // instant; real embeddings-based retrieval is `/api/chat`'s job
    // (see this module's own doc comment).
    let graph = repowise_graph::RepoGraph::build(&index);

    let mut files: Vec<(usize, String)> = index
        .files
        .iter()
        .filter_map(|f| {
            let rel = relative(&state.root, &f.path);
            rel.to_lowercase()
                .contains(&needle)
                .then(|| (graph.dependents_of(&f.path).len(), rel))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    files.truncate(SEARCH_LIMIT);
    let files: Vec<String> = files.into_iter().map(|(_, rel)| rel).collect();

    let mut symbols: Vec<(usize, SymbolDto)> = index
        .files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .filter(|s| s.name.to_lowercase().contains(&needle))
        .map(|s| {
            let dto = SymbolDto {
                name: s.name.clone(),
                kind: s.kind.label().to_string(),
                file: relative(&state.root, &s.file),
                start_line: s.start_line,
            };
            (graph.call_in_degree(&s.id), dto)
        })
        .collect();
    symbols.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    symbols.truncate(SEARCH_LIMIT);
    let symbols: Vec<SymbolDto> = symbols.into_iter().map(|(_, dto)| dto).collect();

    Ok(Json(SearchDto { files, symbols }))
}

async fn get_graph(State(state): State<AppState>) -> Result<Json<GraphDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let graph = repowise_graph::RepoGraph::build(&index);

    let mut ranked: Vec<(&repowise_core::FileRecord, usize)> = index
        .files
        .iter()
        .map(|f| {
            let degree = graph.dependencies_of(&f.path).len() + graph.dependents_of(&f.path).len();
            (f, degree)
        })
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.path.cmp(&b.0.path)));
    let truncated = ranked.len() > GRAPH_NODE_LIMIT;
    ranked.truncate(GRAPH_NODE_LIMIT);

    let included: std::collections::HashSet<&Path> =
        ranked.iter().map(|(f, _)| f.path.as_path()).collect();

    let nodes = ranked
        .iter()
        .map(|(f, _)| GraphNodeDto {
            id: relative(&state.root, &f.path),
            language: f.language.label().to_string(),
        })
        .collect();

    let mut edges = Vec::new();
    for (f, _) in &ranked {
        for dep in graph.dependencies_of(&f.path) {
            if included.contains(dep.as_path()) {
                edges.push(GraphEdgeDto {
                    from: relative(&state.root, &f.path),
                    to: relative(&state.root, &dep),
                });
            }
        }
    }

    Ok(Json(GraphDto {
        nodes,
        edges,
        truncated,
    }))
}

/// `path` is matched against the indexed-files set (not joined onto
/// `root` directly) before ever reaching `git blame`, the same
/// path-traversal-safe convention `/api/wiki` uses.
async fn get_ownership(
    State(state): State<AppState>,
    Query(query): Query<OwnershipQuery>,
) -> Result<Json<OwnershipDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let Some(file) = index
        .files
        .iter()
        .find(|f| relative(&state.root, &f.path) == query.path)
    else {
        return Ok(Json(OwnershipDto {
            available: false,
            owners: Vec::new(),
        }));
    };

    let dto = match repowise_git::ownership_of(&state.root, &file.path) {
        Ok(owners) => OwnershipDto {
            available: true,
            owners: owners
                .into_iter()
                .map(|o| OwnershipEntryDto {
                    author: o.author,
                    lines: o.lines,
                    percentage: o.percentage,
                })
                .collect(),
        },
        Err(_) => OwnershipDto {
            available: false,
            owners: Vec::new(),
        },
    };
    Ok(Json(dto))
}

async fn get_symbol_detail(
    State(state): State<AppState>,
    Query(query): Query<SymbolDetailQuery>,
) -> Result<Json<SymbolDetailDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let found = index.files.iter().find_map(|f| {
        (relative(&state.root, &f.path) == query.file)
            .then(|| f.symbols.iter().find(|s| s.start_line == query.line))
            .flatten()
            .map(|s| (f, s))
    });
    // An unknown symbol is a reportable state, not a 404 -- the client
    // renders a not-found view rather than an error banner.
    let Some((file, sym)) = found else {
        return Ok(Json(SymbolDetailDto {
            found: false,
            name: String::new(),
            kind: String::new(),
            file: query.file,
            start_line: query.line,
            end_line: 0,
            parent: None,
            complexity: 0,
            max_nesting_depth: 0,
            callees: Vec::new(),
            callers: Vec::new(),
            unresolved_callee_count: 0,
        }));
    };

    // Callees come from the symbol's own recorded calls; a call whose
    // target no symbol matches is counted rather than listed, so an
    // empty list can't be misread as "this calls nothing".
    let mut callees: Vec<String> = Vec::new();
    let mut unresolved_callee_count = 0usize;
    for call in &file.calls {
        if call.caller.as_deref() != Some(sym.id.as_str()) {
            continue;
        }
        match index
            .files
            .iter()
            .flat_map(|f| &f.symbols)
            .find(|s| s.name == call.callee_name)
        {
            Some(target) => callees.push(target.name.clone()),
            None => unresolved_callee_count += 1,
        }
    }
    callees.sort();
    callees.dedup();

    let mut callers: Vec<String> = index
        .files
        .iter()
        .flat_map(|f| f.calls.iter().map(move |c| (f, c)))
        .filter(|(_, c)| c.callee_name == sym.name)
        .filter_map(|(f, c)| {
            let id = c.caller.as_deref()?;
            f.symbols
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.name.clone())
        })
        .collect();
    callers.sort();
    callers.dedup();

    Ok(Json(SymbolDetailDto {
        found: true,
        name: sym.name.clone(),
        kind: sym.kind.label().to_string(),
        file: relative(&state.root, &file.path),
        start_line: sym.start_line,
        end_line: sym.end_line,
        parent: sym.parent.clone(),
        complexity: sym.complexity,
        max_nesting_depth: sym.max_nesting_depth,
        callees,
        callers,
        unresolved_callee_count,
    }))
}

async fn get_decision_detail(
    State(state): State<AppState>,
    Query(query): Query<DecisionDetailQuery>,
) -> Result<Json<DecisionDetailDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let decisions = repowise_adr::mine(&index).unwrap_or_default();

    // The reverse lineage link: which decision (if any) this one
    // supersedes. Only derivable by scanning the whole set, since a
    // record only stores the forward `superseded_by`.
    let supersedes = decisions
        .iter()
        .find(|d| d.superseded_by.as_deref() == Some(query.id.as_str()))
        .map(|d| d.id.clone());

    let Some(d) = decisions.into_iter().find(|d| d.id == query.id) else {
        return Ok(Json(DecisionDetailDto {
            found: false,
            id: query.id,
            title: String::new(),
            status: None,
            superseded_by: None,
            supersedes: None,
            body: String::new(),
            linked_files: Vec::new(),
            source: String::new(),
            inferred: false,
        }));
    };

    Ok(Json(DecisionDetailDto {
        found: true,
        source: source_label(&state.root, &d.source),
        inferred: d.source.is_inferred(),
        id: d.id,
        title: d.title,
        status: d.status,
        superseded_by: d.superseded_by,
        supersedes,
        body: d.body,
        linked_files: d
            .linked_files
            .iter()
            .map(|f| relative(&state.root, f))
            .collect(),
    }))
}

async fn get_stats(State(state): State<AppState>) -> Result<Json<StatsDto>, ApiError> {
    let shallow = repowise_git::is_shallow(&state.root);
    // No git history is an empty state, not an error -- same as every
    // other git-backed endpoint here.
    let Ok(commits) = repowise_git::collect_commits(&state.root) else {
        return Ok(Json(StatsDto {
            available: false,
            shallow,
            commit_count: 0,
            punch_card: vec![vec![0; 24]; 7],
            weekly_trend: Vec::new(),
            timezone: "UTC",
        }));
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let timestamps: Vec<i64> = commits.iter().map(|c| c.timestamp).collect();
    let activity = repowise_git::commit_activity(&timestamps, now);

    Ok(Json(StatsDto {
        available: activity.commit_count > 0,
        shallow,
        commit_count: activity.commit_count,
        punch_card: activity.punch_card,
        weekly_trend: activity.weekly_trend,
        timezone: "UTC",
    }))
}

async fn get_files(State(state): State<AppState>) -> Result<Json<FilesDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let graph = repowise_graph::RepoGraph::build(&index);
    let report = repowise_health::analyze(&index, &graph);

    let scores: std::collections::HashMap<&Path, (f64, usize)> = report
        .file_scores
        .iter()
        .map(|f| (f.file.as_path(), (f.score, f.finding_count)))
        .collect();
    let health_available = !scores.is_empty();

    let mut files: Vec<FileEntryDto> = index
        .files
        .iter()
        .map(|f| {
            let scored = scores.get(f.path.as_path());
            FileEntryDto {
                path: relative(&state.root, &f.path),
                language: f.language.label().to_string(),
                lines: f.lines,
                score: scored.map(|(s, _)| *s),
                finding_count: scored.map(|(_, c)| *c).unwrap_or(0),
            }
        })
        .collect();
    // Deterministic order: largest first, path as tiebreak. The treemap
    // layout is a pure function of this order, so a stable order is what
    // stops the view reshuffling between loads.
    files.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.path.cmp(&b.path)));

    Ok(Json(FilesDto {
        total_lines: files.iter().map(|f| f.lines).sum(),
        files,
        health_available,
    }))
}

async fn get_contributors(
    State(state): State<AppState>,
) -> Result<Json<ContributorsDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let files_total = index.files.len();

    // Largest files first: they hold most of the repo's lines, so when
    // the limit bites, the sample retains the shares that matter most.
    let mut by_size: Vec<_> = index.files.iter().collect();
    by_size.sort_by_key(|f| std::cmp::Reverse(f.lines));
    let limit_applied = by_size.len() > CONTRIBUTORS_FILE_LIMIT;
    by_size.truncate(CONTRIBUTORS_FILE_LIMIT);
    let considered = by_size.len();

    let mut lines_by_author: std::collections::BTreeMap<String, usize> = Default::default();
    let mut files_by_author: std::collections::BTreeMap<String, usize> = Default::default();
    let mut bus_factors: std::collections::BTreeMap<usize, usize> = Default::default();
    let mut files_sampled = 0usize;

    for file in by_size {
        // A file that can't be blamed (untracked, or no git at all) is
        // skipped rather than failing the whole endpoint -- consistent
        // with how every other git-backed surface here degrades.
        let Ok(owners) = repowise_git::ownership_of(&state.root, &file.path) else {
            continue;
        };
        if owners.is_empty() {
            continue;
        }
        files_sampled += 1;
        *bus_factors
            .entry(repowise_git::bus_factor(&owners))
            .or_insert(0) += 1;
        for o in owners {
            *lines_by_author.entry(o.author.clone()).or_insert(0) += o.lines;
            *files_by_author.entry(o.author).or_insert(0) += 1;
        }
    }

    let total_lines: usize = lines_by_author.values().sum();
    let mut contributors: Vec<ContributorDto> = lines_by_author
        .into_iter()
        .map(|(author, lines_owned)| ContributorDto {
            percent: if total_lines == 0 {
                0.0
            } else {
                lines_owned as f64 / total_lines as f64 * 100.0
            },
            files_touched: files_by_author.get(&author).copied().unwrap_or(0),
            author,
            lines_owned,
        })
        .collect();
    contributors.sort_by(|a, b| {
        b.lines_owned
            .cmp(&a.lines_owned)
            .then_with(|| a.author.cmp(&b.author))
    });

    Ok(Json(ContributorsDto {
        available: files_sampled > 0,
        contributors,
        bus_factor_distribution: bus_factors.into_iter().collect(),
        files_sampled,
        files_total,
        limit_applied,
        files_unblameable: considered.saturating_sub(files_sampled),
    }))
}

async fn get_coverage(State(state): State<AppState>) -> Result<Json<CoverageDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let Ok(coverage) = repowise_core::coverage::CoverageData::load(&state.root) else {
        return Ok(Json(CoverageDto {
            available: false,
            files: Vec::new(),
            unmeasured_files: index.files.len(),
            mean_percent: 0.0,
            has_per_test_map: false,
            test_contexts: 0,
        }));
    };

    let mut files: Vec<FileCoverageDto> = Vec::new();
    let mut unmeasured_files = 0usize;
    for file in &index.files {
        match coverage.line_coverage_of(&file.path) {
            // `None` means no report ever mentioned this file. Counting
            // it rather than listing it at 0% is the whole point -- see
            // CoverageDto::files.
            None => unmeasured_files += 1,
            Some(percent) => {
                let lines = coverage.files.get(&file.path);
                let lines_known = lines.map(|l| l.len()).unwrap_or(0);
                let lines_hit = lines
                    .map(|l| l.values().filter(|c| **c > 0).count())
                    .unwrap_or(0);
                files.push(FileCoverageDto {
                    path: relative(&state.root, &file.path),
                    percent,
                    lines_known,
                    lines_hit,
                });
            }
        }
    }

    let mean_percent = if files.is_empty() {
        0.0
    } else {
        files.iter().map(|f| f.percent).sum::<f64>() / files.len() as f64
    };
    files.sort_by(|a, b| {
        a.percent
            .partial_cmp(&b.percent)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });

    Ok(Json(CoverageDto {
        // Coverage exists on disk but matched no indexed file: report it
        // as unavailable rather than as a repo with 0 measured files,
        // which would read as "nothing is covered".
        available: !files.is_empty(),
        files,
        unmeasured_files,
        mean_percent,
        has_per_test_map: coverage.has_per_test_map(),
        test_contexts: coverage.per_test.len(),
    }))
}

async fn get_dead_code(
    State(state): State<AppState>,
    Query(query): Query<DeadCodeQuery>,
) -> Result<Json<DeadCodeDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let graph = repowise_graph::RepoGraph::build(&index);
    let candidates = repowise_health::find_dead_code(&index, &graph);

    let threshold = match query.min_confidence.as_deref() {
        None => repowise_health::DeadCodeConfidence::Low,
        Some(s) if s.eq_ignore_ascii_case("low") => repowise_health::DeadCodeConfidence::Low,
        Some(s) if s.eq_ignore_ascii_case("medium") => repowise_health::DeadCodeConfidence::Medium,
        Some(s) if s.eq_ignore_ascii_case("high") => repowise_health::DeadCodeConfidence::High,
        Some(other) => {
            return Err(
                anyhow::anyhow!("min_confidence must be low/medium/high, got {other:?}").into(),
            );
        }
    };

    let matching: Vec<_> = candidates
        .into_iter()
        .filter(|c| c.confidence >= threshold)
        .collect();
    let total_matching = matching.len();
    let candidates = matching
        .into_iter()
        .take(DEAD_CODE_LIMIT)
        .map(|c| DeadCodeCandidateDto {
            file: relative(&state.root, &c.file),
            symbol: c.symbol,
            line: c.line,
            confidence: c.confidence.label().to_string(),
            risk_factors: c.risk_factors,
        })
        .collect();

    Ok(Json(DeadCodeDto {
        candidates,
        total_matching,
    }))
}

async fn post_chat(
    State(state): State<AppState>,
    Json(request): Json<ChatRequestDto>,
) -> Result<Json<ChatResponseDto>, ApiError> {
    let Some(config) = state.llm_config.as_ref().clone() else {
        return Ok(Json(ChatResponseDto {
            available: false,
            reply: None,
            cited: Vec::new(),
            retrieval_mode: String::new(),
            retrieval_caveat: None,
            vectors_reused: None,
            vectors_embedded_now: None,
        }));
    };

    let index = RepoIndex::load(&state.root)?;
    let question = request
        .history
        .iter()
        .rev()
        .find(|t| t.role == "user")
        .map(|t| t.content.clone())
        .unwrap_or_default();
    let root = (*state.root).clone();
    let history: Vec<repowise_llm::Turn> = request
        .history
        .into_iter()
        .map(|t| repowise_llm::Turn {
            role: t.role,
            content: t.content,
        })
        .collect();

    let (completion, retrieval) = tokio::task::spawn_blocking(move || {
        let retrieval = repowise_llm::retrieve(&root, &index, &question, &config);
        let mut turns = vec![repowise_llm::Turn::system(retrieval.context.clone())];
        turns.extend(history);
        (
            repowise_llm::complete_messages_with_usage(&config, &turns),
            retrieval,
        )
    })
    .await
    .map_err(anyhow::Error::from)?;
    let (reply, usage) = completion?;

    if let Some(usage) = usage {
        state.usage.record(usage);
    }

    Ok(Json(ChatResponseDto {
        available: true,
        reply: Some(reply),
        // Surfaced so the dashboard can show sources and flag a degraded
        // retrieval, rather than presenting every answer as equally
        // grounded.
        cited: retrieval.cited,
        retrieval_mode: retrieval.mode.label().to_string(),
        retrieval_caveat: retrieval.mode.caveat().map(str::to_string),
        vectors_reused: retrieval.vectors.map(|v| v.reused),
        vectors_embedded_now: retrieval.vectors.map(|v| v.embedded_now),
    }))
}

async fn get_usage(State(state): State<AppState>) -> Json<UsageTotalsDto> {
    Json(state.usage.snapshot())
}

#[derive(Serialize)]
struct WorkspaceRepoDto {
    name: String,
    path: String,
    indexed: bool,
    file_count: Option<usize>,
    other_file_count: Option<usize>,
}

impl From<repowise_workspace::RepoStatus> for WorkspaceRepoDto {
    fn from(s: repowise_workspace::RepoStatus) -> Self {
        WorkspaceRepoDto {
            name: s.name,
            path: s.path.display().to_string(),
            indexed: s.indexed,
            file_count: s.file_count,
            other_file_count: s.other_file_count,
        }
    }
}

#[derive(Serialize)]
struct WorkspaceReposDto {
    available: bool,
    repos: Vec<WorkspaceRepoDto>,
}

/// Issue #64's first slice: reports every repo the server was started
/// with (`--workspace <path>`), each with its indexed status and file
/// count. `available: false` (empty `repos`) when no workspace was
/// configured -- same degrade-gracefully shape as every other
/// optional-data endpoint in this module.
async fn get_workspace_repos(State(state): State<AppState>) -> Json<WorkspaceReposDto> {
    let dto = match state.workspace_repos.as_ref() {
        Some(repos) => WorkspaceReposDto {
            available: true,
            repos: repos
                .iter()
                .map(repowise_workspace::repo_status)
                .map(WorkspaceRepoDto::from)
                .collect(),
        },
        None => WorkspaceReposDto {
            available: false,
            repos: Vec::new(),
        },
    };
    Json(dto)
}

const WORKSPACE_CO_CHANGE_PAIRS_LIMIT: usize = 10;

#[derive(Serialize)]
struct CoChangePairDto {
    file_a: String,
    file_b: String,
    count: usize,
}

impl From<repowise_workspace::CoChangePair> for CoChangePairDto {
    fn from(p: repowise_workspace::CoChangePair) -> Self {
        CoChangePairDto {
            file_a: p.file_a.display().to_string(),
            file_b: p.file_b.display().to_string(),
            count: p.count,
        }
    }
}

#[derive(Serialize)]
struct RepoCoChangesDto {
    name: String,
    path: String,
    available: bool,
    pairs: Vec<CoChangePairDto>,
}

impl From<repowise_workspace::RepoCoChanges> for RepoCoChangesDto {
    fn from(r: repowise_workspace::RepoCoChanges) -> Self {
        RepoCoChangesDto {
            name: r.name,
            path: r.path.display().to_string(),
            available: r.available,
            pairs: r.pairs.into_iter().map(CoChangePairDto::from).collect(),
        }
    }
}

#[derive(Serialize)]
struct WorkspaceCoChangesDto {
    available: bool,
    repos: Vec<RepoCoChangesDto>,
}

/// Each workspace repo's own most-coupled file pairs, side by side --
/// see `repowise_workspace::workspace_co_changes`'s doc comment for why
/// this isn't literally cross-repo co-change. `available: false` (empty
/// `repos`) when no workspace was configured, same shape as
/// `/api/workspace-repos`.
async fn get_workspace_co_changes(State(state): State<AppState>) -> Json<WorkspaceCoChangesDto> {
    let dto = match state.workspace_repos.as_ref() {
        Some(repos) => WorkspaceCoChangesDto {
            available: true,
            repos: repowise_workspace::workspace_co_changes(repos, WORKSPACE_CO_CHANGE_PAIRS_LIMIT)
                .into_iter()
                .map(RepoCoChangesDto::from)
                .collect(),
        },
        None => WorkspaceCoChangesDto {
            available: false,
            repos: Vec::new(),
        },
    };
    Json(dto)
}

const WORKSPACE_ARCHITECTURE_EDGES_LIMIT: usize = 200;

#[derive(Serialize)]
struct RepoEdgeSummaryDto {
    from_repo: String,
    to_repo: String,
    edge_count: usize,
}

impl From<repowise_workspace::RepoEdgeSummary> for RepoEdgeSummaryDto {
    fn from(e: repowise_workspace::RepoEdgeSummary) -> Self {
        RepoEdgeSummaryDto {
            from_repo: e.from_repo,
            to_repo: e.to_repo,
            edge_count: e.edge_count,
        }
    }
}

#[derive(Serialize)]
struct CrossRepoEdgeDto {
    from_repo: String,
    from_file: String,
    line: usize,
    to_repo: String,
    to_file: String,
    import_path: String,
}

impl From<repowise_graph::CrossRepoImportEdge> for CrossRepoEdgeDto {
    fn from(e: repowise_graph::CrossRepoImportEdge) -> Self {
        CrossRepoEdgeDto {
            from_repo: e.from_repo,
            from_file: e.from_file.display().to_string(),
            line: e.line,
            to_repo: e.to_repo,
            to_file: e.to_file.display().to_string(),
            import_path: e.import_path,
        }
    }
}

#[derive(Serialize)]
struct WorkspaceArchitectureDto {
    available: bool,
    repos: Vec<WorkspaceRepoDto>,
    repo_edges: Vec<RepoEdgeSummaryDto>,
    edges: Vec<CrossRepoEdgeDto>,
    total_edges: usize,
}

/// Real cross-repo Rust `use` resolution across every workspace repo --
/// which repos depend on which others, and the individual import sites
/// behind each dependency. `edges` is capped at
/// `WORKSPACE_ARCHITECTURE_EDGES_LIMIT`; `total_edges` reports the
/// uncapped count. `available: false` (empty lists) when no workspace
/// was configured, same shape as `/api/workspace-repos`.
async fn get_workspace_architecture(
    State(state): State<AppState>,
) -> Json<WorkspaceArchitectureDto> {
    let dto = match state.workspace_repos.as_ref() {
        Some(repos) => {
            let report = repowise_workspace::workspace_architecture(repos);
            let total_edges = report.edges.len();
            let edges = report
                .edges
                .into_iter()
                .take(WORKSPACE_ARCHITECTURE_EDGES_LIMIT)
                .map(CrossRepoEdgeDto::from)
                .collect();
            WorkspaceArchitectureDto {
                available: true,
                repos: report
                    .repos
                    .into_iter()
                    .map(WorkspaceRepoDto::from)
                    .collect(),
                repo_edges: report
                    .repo_edges
                    .into_iter()
                    .map(RepoEdgeSummaryDto::from)
                    .collect(),
                edges,
                total_edges,
            }
        }
        None => WorkspaceArchitectureDto {
            available: false,
            repos: Vec::new(),
            repo_edges: Vec::new(),
            edges: Vec::new(),
            total_edges: 0,
        },
    };
    Json(dto)
}

#[derive(Serialize)]
struct WorkspaceConformanceDto {
    available: bool,
    /// Each entry is one set of repo names involved in a circular
    /// cross-repo dependency (repo A imports repo B imports repo A, or
    /// a longer cycle) -- a workspace's repo-level dependency graph
    /// should form a DAG; a cycle is a concrete, deterministic "pattern
    /// divergence" finding needing no further human-specified rule set.
    /// Empty (not an error) when no cycles are found.
    cycles: Vec<Vec<String>>,
}

/// Circular cross-repo dependencies, reusing exactly the edges
/// `/api/workspace-architecture` already computes -- see
/// `repowise_workspace::detect_workspace_cycles`. `available: false`
/// (empty `cycles`) when no workspace was configured, same shape as
/// every other workspace-wide endpoint.
async fn get_workspace_conformance(State(state): State<AppState>) -> Json<WorkspaceConformanceDto> {
    let dto = match state.workspace_repos.as_ref() {
        Some(repos) => WorkspaceConformanceDto {
            available: true,
            cycles: repowise_workspace::detect_workspace_cycles(repos),
        },
        None => WorkspaceConformanceDto {
            available: false,
            cycles: Vec::new(),
        },
    };
    Json(dto)
}

#[derive(Serialize)]
struct ContractMatchDto {
    producer_repo: String,
    producer_file: String,
    consumer_repo: String,
    consumer_file: String,
    path: String,
}

impl From<repowise_workspace::ContractMatch> for ContractMatchDto {
    fn from(m: repowise_workspace::ContractMatch) -> Self {
        ContractMatchDto {
            producer_repo: m.producer_repo,
            producer_file: m.producer_file.display().to_string(),
            consumer_repo: m.consumer_repo,
            consumer_file: m.consumer_file.display().to_string(),
            path: m.path,
        }
    }
}

#[derive(Serialize)]
struct UnmatchedConsumerDto {
    repo: String,
    file: String,
    path: String,
}

impl From<repowise_workspace::ConsumerCall> for UnmatchedConsumerDto {
    fn from(c: repowise_workspace::ConsumerCall) -> Self {
        UnmatchedConsumerDto {
            repo: c.repo,
            file: c.file.display().to_string(),
            path: c.path,
        }
    }
}

#[derive(Serialize)]
struct WorkspaceContractsDto {
    available: bool,
    matches: Vec<ContractMatchDto>,
    unmatched_consumers: Vec<UnmatchedConsumerDto>,
}

/// Regex-based HTTP producer/consumer route matching across every
/// workspace repo -- see `repowise_workspace::workspace_contracts`'s
/// own doc comment for why this is coarse and heuristic by design
/// (no cross-repo symbol resolution involved, just a fixed pattern
/// table over raw source text). `available: false` (empty lists) when
/// no workspace was configured, same shape as every other workspace
/// endpoint.
async fn get_workspace_contracts(State(state): State<AppState>) -> Json<WorkspaceContractsDto> {
    let dto = match state.workspace_repos.as_ref() {
        Some(repos) => {
            let report = repowise_workspace::workspace_contracts(repos);
            WorkspaceContractsDto {
                available: true,
                matches: report
                    .matches
                    .into_iter()
                    .map(ContractMatchDto::from)
                    .collect(),
                unmatched_consumers: report
                    .unmatched_consumers
                    .into_iter()
                    .map(UnmatchedConsumerDto::from)
                    .collect(),
            }
        }
        None => WorkspaceContractsDto {
            available: false,
            matches: Vec::new(),
            unmatched_consumers: Vec::new(),
        },
    };
    Json(dto)
}

async fn get_settings(State(state): State<AppState>) -> Result<Json<SettingsDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let git_available = repowise_git::GitAnalytics::collect(&state.root).is_ok();
    let wiki_pages_available = !wiki_indexed_files(&state.root, &index).is_empty();
    let llm_config = state.llm_config.as_ref().clone();

    Ok(Json(SettingsDto {
        root: state.root.display().to_string(),
        file_count: index.files.len(),
        other_file_count: index.other_files,
        git_available,
        wiki_pages_available,
        llm_configured: llm_config.is_some(),
        llm_model: llm_config.map(|c| c.model),
    }))
}

/// Kick off a background reindex (`repowise_parser::build_index`, the
/// same implementation `repowise-cli`'s `init`/`update` commands use) if
/// one isn't already running, and return the job's current status.
/// Never errors on a bad root -- a reindex failure surfaces as a
/// `Failed` status for the dashboard to render, not a 500.
async fn post_reindex(State(state): State<AppState>) -> Json<ReindexStatusDto> {
    if state.reindex_job.try_start() {
        let root = (*state.root).clone();
        let job = state.reindex_job.clone();
        tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            let outcome = repowise_parser::build_index(&root).and_then(|index| {
                index.save(&index.root)?;
                Ok((index.files.len(), index.other_files))
            });
            let duration_ms = start.elapsed().as_millis() as u64;
            let status = match outcome {
                Ok((file_count, other_file_count)) => ReindexStatusDto::Completed {
                    file_count,
                    other_file_count,
                    duration_ms,
                },
                Err(e) => ReindexStatusDto::Failed {
                    error: e.to_string(),
                },
            };
            job.finish(status);
        });
    }
    Json(state.reindex_job.snapshot())
}

/// The dashboard polls this to render the live job banner.
async fn get_reindex_status(State(state): State<AppState>) -> Json<ReindexStatusDto> {
    Json(state.reindex_job.snapshot())
}

/// Build the axum `Router` — separated from `serve` so tests can drive
/// requests directly against it (via `tower::ServiceExt::oneshot`)
/// without binding a real socket. `static_dir`, if given, serves the
/// built `repowise-web` frontend (e.g. `crates/repowise-web/dist` after
/// `trunk build`) as a fallback for any path the JSON API doesn't claim.
pub fn app(root: PathBuf, static_dir: Option<PathBuf>, workspace: Option<PathBuf>) -> Router {
    let workspace_repos = workspace.and_then(|path| repowise_workspace::load_resolved(&path).ok());
    let state = AppState {
        root: Arc::new(root),
        llm_config: Arc::new(repowise_llm::LlmConfig::from_env()),
        workspace_repos: Arc::new(workspace_repos),
        reindex_job: ReindexJob::new(),
        usage: UsageTracker::new(),
    };
    build_router(state, static_dir)
}

fn build_router(state: AppState, static_dir: Option<PathBuf>) -> Router {
    let router = Router::new()
        .route("/api/overview", get(get_overview))
        .route("/api/health", get(get_health))
        .route("/api/hotspots", get(get_hotspots))
        .route("/api/decisions", get(get_decisions))
        .route("/api/symbols", get(get_symbols))
        .route("/api/wiki-pages", get(get_wiki_pages))
        .route("/api/wiki", get(get_wiki))
        .route("/api/search", get(get_search))
        .route("/api/graph", get(get_graph))
        .route("/api/ownership", get(get_ownership))
        .route("/api/symbol", get(get_symbol_detail))
        .route("/api/decision", get(get_decision_detail))
        .route("/api/stats", get(get_stats))
        .route("/api/files", get(get_files))
        .route("/api/contributors", get(get_contributors))
        .route("/api/coverage", get(get_coverage))
        .route("/api/dead-code", get(get_dead_code))
        .route("/api/chat", post(post_chat))
        .route("/api/reindex", get(get_reindex_status).post(post_reindex))
        .route("/api/settings", get(get_settings))
        .route("/api/usage", get(get_usage))
        .route("/api/workspace-repos", get(get_workspace_repos))
        .route("/api/workspace-co-changes", get(get_workspace_co_changes))
        .route(
            "/api/workspace-architecture",
            get(get_workspace_architecture),
        )
        .route("/api/workspace-conformance", get(get_workspace_conformance))
        .route("/api/workspace-contracts", get(get_workspace_contracts))
        .with_state(state);
    match static_dir {
        Some(dir) => router.fallback_service(ServeDir::new(dir)),
        None => router,
    }
}

/// Bind `addr` and serve `app(root, static_dir, workspace)` until the
/// process is killed. `repowise-cli` drives this from a
/// `tokio::runtime::Runtime` it builds just for this command, the same
/// "rest of the CLI stays synchronous" pattern `repowise serve` (the
/// MCP server) already uses.
pub async fn serve(
    root: PathBuf,
    addr: SocketAddr,
    static_dir: Option<PathBuf>,
    workspace: Option<PathBuf>,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app(root, static_dir, workspace)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn get_overview_returns_json_matching_the_indexed_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();

        let discovered = repowise_core::discover_files(&root).unwrap();
        let mut files = Vec::new();
        let mut other_files = 0;
        for entry in discovered {
            if matches!(entry.language, repowise_core::Language::Other) {
                other_files += 1;
                continue;
            }
            let source = std::fs::read_to_string(&entry.path).unwrap();
            // repowise-server doesn't depend on repowise-parser (it's
            // not needed for anything beyond loading an already-built
            // index), so this test builds a minimal FileRecord by hand
            // instead of parsing -- good enough to exercise the JSON path.
            files.push(repowise_core::FileRecord {
                path: entry.path,
                language: entry.language,
                lines: source.lines().count(),
                symbols: vec![],
                imports: vec![],
                calls: vec![],
                field_accesses: vec![],
            });
        }
        let index = RepoIndex {
            root: root.clone(),
            files,
            other_files,
            indexed_commit: None,
        };
        index.save(&root).unwrap();

        let response = app(root, None, None)
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/overview")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["file_count"], 1);
        assert_eq!(json["other_file_count"], 0);
    }

    #[tokio::test]
    async fn get_overview_returns_a_server_error_without_a_prior_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let response = app(root, None, None)
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/overview")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// A repo with one file containing one over-threshold-complexity
    /// symbol -- enough to trigger a `high-complexity` health finding
    /// and to exercise `/api/symbols` with real symbol data.
    fn index_with_one_busy_symbol(root: &Path) -> RepoIndex {
        let file = root.join("busy.rs");
        std::fs::write(&file, "pub fn busy() {}\n").unwrap();
        let symbol = repowise_core::Symbol {
            id: "busy.rs::busy::1".to_string(),
            name: "busy".to_string(),
            kind: repowise_core::SymbolKind::Function,
            file: file.clone(),
            start_line: 1,
            end_line: 1,
            parent: None,
            complexity: repowise_health::HIGH_COMPLEXITY + 1,
            max_nesting_depth: 0,
            bumpy_road_bumps: 0,
            complex_conditionals: Vec::new(),
            io_in_loop: Vec::new(),
            string_concat_in_loop: Vec::new(),
            resource_construction_in_loop: Vec::new(),
            lock_in_loop: Vec::new(),
            list_insert_zero_in_loop: Vec::new(),
            json_parse_in_loop: Vec::new(),
            regex_compile_in_loop: Vec::new(),
            nested_loop_with_io: Vec::new(),
            nested_loop_quadratic: Vec::new(),
            serial_await_in_loop: Vec::new(),
            pd_concat_in_loop: Vec::new(),
            blocking_sync_in_async: Vec::new(),
            blocking_io_under_lock: Vec::new(),
            array_spread_in_reduce: Vec::new(),
            sql_cartesian_join: Vec::new(),
            defer_in_loop: Vec::new(),
            goroutine_in_unbounded_loop: Vec::new(),
            membership_test_in_loop: Vec::new(),
            sync_io_calls: Vec::new(),
            param_count: 0,
            primitive_param_count: 0,
            body_hash: None,
        };
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![repowise_core::FileRecord {
                path: file,
                language: repowise_core::Language::Rust,
                lines: 1,
                symbols: vec![symbol],
                imports: vec![],
                calls: vec![],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    /// A repo with two files where `a.rs` imports `b.rs` -- enough to
    /// exercise `/api/graph`'s nodes/edges without depending on any
    /// language-specific import-path resolution heuristic (the import
    /// is pre-resolved via `ImportRef::resolved_file`, same as a real
    /// parser would set for e.g. Rust's `mod foo;`).
    fn index_with_one_import_edge(root: &Path) -> RepoIndex {
        let a = root.join("a.rs");
        let b = root.join("b.rs");
        std::fs::write(&a, "mod b;\n").unwrap();
        std::fs::write(&b, "pub fn helper() {}\n").unwrap();
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![
                repowise_core::FileRecord {
                    path: a.clone(),
                    language: repowise_core::Language::Rust,
                    lines: 1,
                    symbols: vec![],
                    imports: vec![repowise_core::ImportRef {
                        path: "b".to_string(),
                        line: 1,
                        resolved_file: Some(b.clone()),
                    }],
                    calls: vec![],
                    field_accesses: vec![],
                },
                repowise_core::FileRecord {
                    path: b,
                    language: repowise_core::Language::Rust,
                    lines: 1,
                    symbols: vec![],
                    imports: vec![],
                    calls: vec![],
                    field_accesses: vec![],
                },
            ],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    /// A caller (`task_aardvark`, never called) and a callee
    /// (`task_zebra`, called once) sharing a `task_` name prefix so a
    /// single search matches both -- named so plain alphabetical order
    /// would rank the caller first, letting a test distinguish "still
    /// alphabetical" from "actually re-ranked by call in-degree".
    fn index_with_a_caller_and_a_more_called_symbol(root: &Path) -> RepoIndex {
        let file = root.join("lib.rs");
        std::fs::write(
            &file,
            "fn task_aardvark() { task_zebra(); }\nfn task_zebra() {}\n",
        )
        .unwrap();
        let symbol = |id: &str, name: &str, line: usize| repowise_core::Symbol {
            id: id.to_string(),
            name: name.to_string(),
            kind: repowise_core::SymbolKind::Function,
            file: file.clone(),
            start_line: line,
            end_line: line,
            parent: None,
            complexity: 1,
            max_nesting_depth: 0,
            bumpy_road_bumps: 0,
            complex_conditionals: Vec::new(),
            io_in_loop: Vec::new(),
            string_concat_in_loop: Vec::new(),
            resource_construction_in_loop: Vec::new(),
            lock_in_loop: Vec::new(),
            list_insert_zero_in_loop: Vec::new(),
            json_parse_in_loop: Vec::new(),
            regex_compile_in_loop: Vec::new(),
            nested_loop_with_io: Vec::new(),
            nested_loop_quadratic: Vec::new(),
            serial_await_in_loop: Vec::new(),
            pd_concat_in_loop: Vec::new(),
            blocking_sync_in_async: Vec::new(),
            blocking_io_under_lock: Vec::new(),
            array_spread_in_reduce: Vec::new(),
            sql_cartesian_join: Vec::new(),
            defer_in_loop: Vec::new(),
            goroutine_in_unbounded_loop: Vec::new(),
            membership_test_in_loop: Vec::new(),
            sync_io_calls: Vec::new(),
            param_count: 0,
            primitive_param_count: 0,
            body_hash: None,
        };
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![repowise_core::FileRecord {
                path: file.clone(),
                language: repowise_core::Language::Rust,
                lines: 2,
                symbols: vec![
                    symbol("lib.rs::task_aardvark::1", "task_aardvark", 1),
                    symbol("lib.rs::task_zebra::2", "task_zebra", 2),
                ],
                imports: vec![],
                calls: vec![repowise_core::CallRef {
                    caller: Some("lib.rs::task_aardvark::1".to_string()),
                    callee_name: "task_zebra".to_string(),
                    line: 1,
                }],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    async fn get(root: PathBuf, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = app(root, None, None)
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json = if body.is_empty() {
            serde_json::Value::Null
        } else {
            // Error responses carry a plain-text body, not JSON --
            // callers checking those only care about `status`.
            serde_json::from_slice(&body).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&body).into_owned())
            })
        };
        (status, json)
    }

    #[tokio::test]
    async fn get_health_summarizes_findings_and_lists_worst_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/health").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["file_count"], 1);
        // The unused, over-threshold-complexity symbol trips both a
        // high-complexity and a possibly-dead-code finding.
        assert_eq!(json["finding_count"], 2);
        let kinds: Vec<&str> = json["by_kind"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"high-complexity"));
        assert_eq!(json["worst_files"][0]["file"], "busy.rs");
        assert!(json["worst_files"][0]["score"].as_f64().unwrap() < 10.0);
    }

    #[tokio::test]
    async fn get_hotspots_reports_unavailable_without_git_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/hotspots").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["hotspots"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_decisions_is_an_empty_list_when_none_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/decisions").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["decisions"], serde_json::json!([]));
        // The reason an empty list isn't the whole response: "no
        // inferred decisions here" and "the pass that infers them never
        // ran" are different facts, and this repo is the second.
        assert!(json["inferred_source"].as_str().unwrap().contains("opt-in"));
    }

    #[tokio::test]
    async fn get_symbols_returns_every_symbol_with_a_relative_file_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/symbols").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["name"], "busy");
        assert_eq!(json[0]["kind"], "function");
        assert_eq!(json[0]["file"], "busy.rs");
        assert_eq!(json[0]["start_line"], 1);
    }

    #[tokio::test]
    async fn get_wiki_pages_lists_only_files_with_a_wiki_page_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root.clone(), "/api/wiki-pages").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!([]));

        let wiki_path = repowise_docs::wiki_page_path(&root, &root.join("busy.rs"));
        std::fs::create_dir_all(wiki_path.parent().unwrap()).unwrap();
        std::fs::write(&wiki_path, "# busy.rs\n").unwrap();

        let (status, json) = get(root, "/api/wiki-pages").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!(["busy.rs"]));
    }

    #[tokio::test]
    async fn get_wiki_returns_page_content_for_an_indexed_file_with_a_wiki_page() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        let wiki_path = repowise_docs::wiki_page_path(&root, &root.join("busy.rs"));
        std::fs::create_dir_all(wiki_path.parent().unwrap()).unwrap();
        std::fs::write(&wiki_path, "# busy.rs\n\nSome notes.\n").unwrap();

        let (status, json) = get(root, "/api/wiki?path=busy.rs").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["path"], "busy.rs");
        assert_eq!(json["content"], "# busy.rs\n\nSome notes.\n");
    }

    #[tokio::test]
    async fn get_wiki_is_not_found_for_a_path_with_no_wiki_page() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, _json) = get(root, "/api/wiki?path=busy.rs").await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_wiki_is_not_found_for_a_path_traversal_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        let wiki_path = repowise_docs::wiki_page_path(&root, &root.join("busy.rs"));
        std::fs::create_dir_all(wiki_path.parent().unwrap()).unwrap();
        std::fs::write(&wiki_path, "# busy.rs\n").unwrap();

        let (status, _json) = get(root, "/api/wiki?path=..%2F..%2F..%2Fetc%2Fpasswd").await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_search_matches_files_and_symbols_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/search?q=BUSY").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["files"], serde_json::json!(["busy.rs"]));
        assert_eq!(json["symbols"][0]["name"], "busy");
    }

    #[tokio::test]
    async fn get_search_returns_nothing_for_an_empty_query() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/search?q=").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["files"], serde_json::json!([]));
        assert_eq!(json["symbols"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_search_ranks_files_by_dependents_count() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_import_edge(&root);

        // "a.rs" imports "b.rs", so "b.rs" has one dependent and "a.rs"
        // has none; plain alphabetical order would put "a.rs" first.
        let (status, json) = get(root, "/api/search?q=rs").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["files"], serde_json::json!(["b.rs", "a.rs"]));
    }

    #[tokio::test]
    async fn get_search_ranks_symbols_by_call_in_degree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_a_caller_and_a_more_called_symbol(&root);

        let (status, json) = get(root, "/api/search?q=task_").await;

        assert_eq!(status, StatusCode::OK);
        let names: Vec<&str> = json["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["task_zebra", "task_aardvark"]);
    }

    #[tokio::test]
    async fn get_graph_returns_nodes_and_edges_for_an_import() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_import_edge(&root);

        let (status, json) = get(root, "/api/graph").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["truncated"], false);
        let nodes: Vec<&str> = json["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert!(nodes.contains(&"a.rs"));
        assert!(nodes.contains(&"b.rs"));
        assert_eq!(
            json["edges"],
            serde_json::json!([{"from": "a.rs", "to": "b.rs"}])
        );
    }

    #[tokio::test]
    async fn get_graph_has_no_edges_for_a_file_with_no_imports() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/graph").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["edges"], serde_json::json!([]));
        assert_eq!(json["nodes"][0]["id"], "busy.rs");
        assert_eq!(json["nodes"][0]["language"], "Rust");
    }

    /// A repo with one file containing one commented, uncommented-name
    /// symbol -- a `// This is a decision: ...` comment immediately
    /// above it is enough for `repowise_adr::mine`'s code-comment source
    /// to produce a `DecisionRecord` linked to that exact file.
    fn index_with_a_decision_comment(root: &Path) -> RepoIndex {
        let file = root.join("busy.rs");
        std::fs::write(
            &file,
            "// This is a decision: chose recursion for simplicity.\npub fn busy() {}\n",
        )
        .unwrap();
        let symbol = repowise_core::Symbol {
            id: "busy.rs::busy::2".to_string(),
            name: "busy".to_string(),
            kind: repowise_core::SymbolKind::Function,
            file: file.clone(),
            start_line: 2,
            end_line: 2,
            parent: None,
            complexity: 1,
            max_nesting_depth: 0,
            bumpy_road_bumps: 0,
            complex_conditionals: Vec::new(),
            io_in_loop: Vec::new(),
            string_concat_in_loop: Vec::new(),
            resource_construction_in_loop: Vec::new(),
            lock_in_loop: Vec::new(),
            list_insert_zero_in_loop: Vec::new(),
            json_parse_in_loop: Vec::new(),
            regex_compile_in_loop: Vec::new(),
            nested_loop_with_io: Vec::new(),
            nested_loop_quadratic: Vec::new(),
            serial_await_in_loop: Vec::new(),
            pd_concat_in_loop: Vec::new(),
            blocking_sync_in_async: Vec::new(),
            blocking_io_under_lock: Vec::new(),
            array_spread_in_reduce: Vec::new(),
            sql_cartesian_join: Vec::new(),
            defer_in_loop: Vec::new(),
            goroutine_in_unbounded_loop: Vec::new(),
            membership_test_in_loop: Vec::new(),
            sync_io_calls: Vec::new(),
            param_count: 0,
            primitive_param_count: 0,
            body_hash: None,
        };
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![repowise_core::FileRecord {
                path: file,
                language: repowise_core::Language::Rust,
                lines: 2,
                symbols: vec![symbol],
                imports: vec![],
                calls: vec![],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    fn git_commit_all(root: &Path, message: &str) {
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "-q"]);
        // Local config, not env vars: CI runners have neither a global
        // git identity nor GIT_AUTHOR_*/GIT_COMMITTER_* set, so without
        // this the commit fails outright ("empty ident name"). `--author`
        // below still overrides these for the *author* field specifically,
        // giving a deterministic name for ownership assertions regardless
        // of whichever identity a given environment happens to default to.
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
        run(&["add", "-A"]);
        run(&[
            "commit",
            "-q",
            "-m",
            message,
            "--author=Test <test@example.com>",
        ]);
    }

    #[tokio::test]
    async fn get_decisions_filters_by_file_query_param() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_a_decision_comment(&root);

        let (status, json) = get(root.clone(), "/api/decisions").await;
        assert_eq!(status, StatusCode::OK);
        let all = json["decisions"].as_array().unwrap();
        assert_eq!(all.len(), 1);
        // A comment-sourced decision must not be flagged as inferred --
        // the flag is only meaningful if it's exclusive to the one
        // source that isn't a written artifact.
        assert_eq!(all[0]["inferred"], serde_json::json!(false));
        assert!(all[0]["source"].as_str().unwrap().starts_with("comment:"));

        let (status, json) = get(root.clone(), "/api/decisions?file=busy.rs").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["decisions"].as_array().unwrap().len(), 1);

        let (status, json) = get(root, "/api/decisions?file=other.rs").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["decisions"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_ownership_returns_owner_breakdown_for_an_indexed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        git_commit_all(&root, "add busy.rs");

        let (status, json) = get(root, "/api/ownership?path=busy.rs").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], true);
        assert_eq!(json["owners"][0]["author"], "Test");
        assert!(json["owners"][0]["percentage"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn get_ownership_is_unavailable_without_git_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/ownership?path=busy.rs").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["owners"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_ownership_is_unavailable_for_an_unindexed_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        git_commit_all(&root, "add busy.rs");

        let (status, json) = get(root, "/api/ownership?path=nope.rs").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
    }

    #[tokio::test]
    async fn get_settings_reports_root_counts_and_no_llm_or_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root.clone(), "/api/settings").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["root"], root.display().to_string());
        assert_eq!(json["file_count"], 1);
        assert_eq!(json["git_available"], false);
        assert_eq!(json["wiki_pages_available"], false);
        assert_eq!(json["llm_configured"], false);
        assert_eq!(json["llm_model"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn get_settings_reports_git_and_llm_availability_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        git_commit_all(&root, "add busy.rs");

        let config = repowise_llm::LlmConfig {
            base_url: "http://127.0.0.1:0".to_string(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };
        let router = app_with_llm_config(root, Some(config));
        let (status, json) = get_on(router, "/api/settings").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["git_available"], true);
        assert_eq!(json["llm_configured"], true);
        assert_eq!(json["llm_model"], "smart");
    }

    #[tokio::test]
    async fn get_contributors_reports_unavailable_without_git_history() {
        // A temp dir is not a git repo, so every blame fails. That must
        // be an empty state, not a 500 -- consistent with the other
        // git-backed endpoints.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/contributors").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["contributors"].as_array().unwrap().len(), 0);
        // files_total still reports the real index size, so the UI can
        // distinguish "no git" from "empty repo".
        assert_eq!(json["files_total"], 1);
        assert_eq!(json["files_sampled"], 0);
    }

    #[tokio::test]
    async fn get_symbol_detail_reports_not_found_for_an_unknown_symbol() {
        // A stale deep link must render a not-found view, not an error.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/symbol?file=nope.rs&line=99").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["found"], false);
    }

    #[tokio::test]
    async fn get_symbol_detail_returns_the_symbol_and_its_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/symbol?file=busy.rs&line=1").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["found"], true);
        assert_eq!(json["name"], "busy");
        assert_eq!(json["kind"], "function");
        assert!(json["complexity"].as_u64().unwrap() > 0);
        // Nothing calls it and it calls nothing, but both are still
        // well-formed arrays rather than nulls.
        assert!(json["callers"].is_array());
        assert!(json["callees"].is_array());
    }

    #[tokio::test]
    async fn get_decision_detail_reports_not_found_for_an_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/decision?id=ADR-9999").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["found"], false);
        assert_eq!(json["id"], "ADR-9999");
    }

    #[tokio::test]
    async fn get_stats_reports_unavailable_without_git_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/stats").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["commit_count"], 0);
        // The punch card is still well-formed so the client can render
        // an empty grid rather than special-casing a missing field.
        assert_eq!(json["punch_card"].as_array().unwrap().len(), 7);
        // Timezone is always stated, even with no data.
        assert_eq!(json["timezone"], "UTC");
    }

    #[tokio::test]
    async fn get_coverage_reports_unavailable_when_nothing_was_ingested() {
        // Must not read as "0% covered everywhere" -- a repo that never
        // ran `coverage add` is unmeasured, not untested.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/coverage").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["files"].as_array().unwrap().len(), 0);
        assert_eq!(json["unmeasured_files"], 1);
    }

    #[tokio::test]
    async fn get_coverage_separates_measured_files_from_unmeasured_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let index = index_with_one_busy_symbol(&root);
        // Measure busy.rs at 50%; leave any other indexed file absent
        // from the report entirely.
        let report = format!(
            "TN:t\nSF:{}\nDA:1,1\nDA:2,0\nend_of_record\n",
            index.files[0].path.display()
        );
        let (data, _) = repowise_core::coverage::ingest(&report, &root).unwrap();
        data.save(&root).unwrap();

        let (status, json) = get(root, "/api/coverage").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], true);
        let files = json["files"].as_array().unwrap();
        assert_eq!(files.len(), 1, "{files:#?}");
        assert_eq!(files[0]["percent"], 50.0);
        assert_eq!(files[0]["lines_known"], 2);
        assert_eq!(files[0]["lines_hit"], 1);
        assert_eq!(json["unmeasured_files"], 0);
        assert_eq!(json["has_per_test_map"], true);
        assert_eq!(json["test_contexts"], 1);
    }

    #[tokio::test]
    async fn get_dead_code_returns_candidates_for_an_uncalled_function() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/dead-code").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["candidates"][0]["symbol"], "busy");
        assert_eq!(json["total_matching"], 1);
    }

    #[tokio::test]
    async fn get_dead_code_filters_by_min_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/dead-code?min_confidence=high").await;

        assert_eq!(status, StatusCode::OK);
        // "busy" is a unique name and no unresolved import's stem
        // matches its file stem, so it's High confidence -- still
        // included at the high-only threshold.
        assert_eq!(json["total_matching"], 1);
    }

    #[tokio::test]
    async fn get_dead_code_errors_on_invalid_min_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, _json) = get(root, "/api/dead-code?min_confidence=nonsense").await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Same hand-rolled fixture-server approach `repowise-llm`'s own
    /// tests use: a real HTTP round trip with no mocking crate.
    /// Reads a full HTTP request off `stream`: a single `read()` call
    /// isn't guaranteed to return the whole request when the body spans
    /// more than one TCP segment (true for the chat-completions request
    /// once its context includes several files' worth of symbols), so
    /// this loops until it has read the `Content-Length`-declared body
    /// in full (or gives up after a short idle gap, for a request with
    /// no body).
    fn read_full_request(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(500)))
            .ok();
        let mut data = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    data.extend_from_slice(&buf[..n]);
                    let Some(headers_end) = find_subslice(&data, b"\r\n\r\n") else {
                        continue;
                    };
                    let body_start = headers_end + 4;
                    match content_length(&data[..headers_end]) {
                        Some(len) if data.len() < body_start + len => continue,
                        _ => break,
                    }
                }
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn content_length(headers: &[u8]) -> Option<usize> {
        String::from_utf8_lossy(headers).lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
    }

    struct ChatFixtureServer {
        addr: std::net::SocketAddr,
        received: Arc<Mutex<Vec<String>>>,
    }

    impl ChatFixtureServer {
        /// Serves `responses` in order, one per connection -- e.g.
        /// `[embeddings_response, chat_response]` for a chat call that
        /// embeds first and then completes. Every request's raw bytes
        /// are recorded (`requests()`) so a test can assert on which
        /// context (keyword vs. embeddings) was actually sent to the
        /// chat-completions call.
        fn start_sequence(responses: Vec<&'static str>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let received = Arc::new(Mutex::new(Vec::new()));
            let received_for_thread = received.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                for body in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    received_for_thread
                        .lock()
                        .unwrap()
                        .push(read_full_request(&mut stream));
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            ChatFixtureServer { addr, received }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        fn requests(&self) -> Vec<String> {
            self.received.lock().unwrap().clone()
        }
    }

    fn app_with_llm_config(root: PathBuf, llm_config: Option<repowise_llm::LlmConfig>) -> Router {
        let state = AppState {
            root: Arc::new(root),
            llm_config: Arc::new(llm_config),
            workspace_repos: Arc::new(None),
            reindex_job: ReindexJob::new(),
            usage: UsageTracker::new(),
        };
        build_router(state, None)
    }

    async fn post_json(
        router: Router,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json = if body.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&body).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&body).into_owned())
            })
        };
        (status, json)
    }

    #[tokio::test]
    async fn post_chat_reports_unavailable_without_llm_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let router = app_with_llm_config(root, None);
        let (status, json) = post_json(
            router,
            "/api/chat",
            serde_json::json!({"history": [{"role": "user", "content": "hi"}]}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["reply"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn post_chat_returns_a_reply_when_llm_is_configured() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let embeddings_response =
            r#"{"data": [{"embedding": [1.0, 0.0]}, {"embedding": [1.0, 0.0]}]}"#;
        let chat_response = r#"{"choices": [{"message": {"role": "assistant", "content": "busy() lives in busy.rs."}}]}"#;
        let server = ChatFixtureServer::start_sequence(vec![embeddings_response, chat_response]);
        let config = repowise_llm::LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let router = app_with_llm_config(root, Some(config));
        let (status, json) = post_json(
            router,
            "/api/chat",
            serde_json::json!({"history": [{"role": "user", "content": "What does busy do?"}]}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], true);
        assert_eq!(json["reply"], "busy() lives in busy.rs.");
        assert!(server.requests()[1].contains("semantic (embedding) search"));
    }

    #[tokio::test]
    async fn get_usage_starts_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/usage").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["chat_call_count"], 0);
        assert_eq!(json["total_tokens"], 0);
    }

    #[tokio::test]
    async fn get_workspace_repos_is_unavailable_without_a_workspace_flag() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/workspace-repos").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["repos"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_workspace_repos_reports_indexed_and_unindexed_repos() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let indexed_repo = dir.path().join("indexed");
        let unindexed_repo = dir.path().join("unindexed");
        std::fs::create_dir_all(&indexed_repo).unwrap();
        std::fs::create_dir_all(&unindexed_repo).unwrap();
        index_with_one_busy_symbol(&indexed_repo);
        let workspace_path = dir.path().join("workspace.toml");
        std::fs::write(
            &workspace_path,
            format!(
                r#"
                    [[repo]]
                    name = "indexed"
                    path = "{}"

                    [[repo]]
                    name = "unindexed"
                    path = "{}"
                "#,
                indexed_repo.display(),
                unindexed_repo.display(),
            ),
        )
        .unwrap();

        let router = app(root, None, Some(workspace_path));
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/workspace-repos")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["available"], true);
        let repos = json["repos"].as_array().unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0]["name"], "indexed");
        assert_eq!(repos[0]["indexed"], true);
        assert_eq!(repos[0]["file_count"], 1);
        assert_eq!(repos[1]["name"], "unindexed");
        assert_eq!(repos[1]["indexed"], false);
        assert_eq!(repos[1]["file_count"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn get_workspace_co_changes_is_unavailable_without_a_workspace_flag() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/workspace-co-changes").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["repos"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_workspace_co_changes_reports_coupled_files_per_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let member_repo = dir.path().join("member");
        std::fs::create_dir_all(&member_repo).unwrap();
        std::fs::write(member_repo.join("a.txt"), "a\n").unwrap();
        std::fs::write(member_repo.join("b.txt"), "b\n").unwrap();
        git_commit_all(&member_repo, "add a and b together");

        let no_git_repo = dir.path().join("no-git");
        std::fs::create_dir_all(&no_git_repo).unwrap();

        let workspace_path = dir.path().join("workspace.toml");
        std::fs::write(
            &workspace_path,
            format!(
                r#"
                    [[repo]]
                    name = "member"
                    path = "{}"

                    [[repo]]
                    name = "no-git"
                    path = "{}"
                "#,
                member_repo.display(),
                no_git_repo.display(),
            ),
        )
        .unwrap();

        let router = app(root, None, Some(workspace_path));
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/workspace-co-changes")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["available"], true);
        let repos = json["repos"].as_array().unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0]["name"], "member");
        assert_eq!(repos[0]["available"], true);
        assert_eq!(repos[0]["pairs"][0]["count"], 1);
        assert_eq!(repos[1]["name"], "no-git");
        assert_eq!(repos[1]["available"], false);
        assert_eq!(repos[1]["pairs"], serde_json::json!([]));
    }

    /// Writes a minimal real Rust crate on disk (a `Cargo.toml` +
    /// `src/foo.rs` defining `bar()`) and saves an index for it -- the
    /// file's real on-disk location matters here since
    /// `repowise_graph::modpath::rust_module_path` scans the filesystem
    /// for the nearest `Cargo.toml`, independent of anything in the
    /// `RepoIndex` itself.
    fn index_rust_crate_with_foo_bar(root: &Path) -> RepoIndex {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"repo-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let file = root.join("src/foo.rs");
        std::fs::write(&file, "pub fn bar() -> i32 { 42 }\n").unwrap();
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![repowise_core::FileRecord {
                path: file,
                language: repowise_core::Language::Rust,
                lines: 1,
                symbols: vec![],
                imports: vec![],
                calls: vec![],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    /// Writes a minimal real Rust crate importing `repo_a::foo::bar` --
    /// the `ImportRef` is hand-constructed (unresolved, exactly as a
    /// real `use` statement would parse) rather than parsed from source,
    /// consistent with this file's established "no `repowise-parser`
    /// dependency" convention.
    fn index_rust_crate_importing_repo_a(root: &Path) -> RepoIndex {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"repo-b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let file = root.join("src/lib.rs");
        std::fs::write(&file, "use repo_a::foo::bar;\n").unwrap();
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![repowise_core::FileRecord {
                path: file,
                language: repowise_core::Language::Rust,
                lines: 1,
                symbols: vec![],
                imports: vec![repowise_core::ImportRef {
                    path: "repo_a::foo::bar".to_string(),
                    line: 1,
                    resolved_file: None,
                }],
                calls: vec![],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    #[tokio::test]
    async fn get_workspace_architecture_is_unavailable_without_a_workspace_flag() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/workspace-architecture").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["edges"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_workspace_architecture_reports_cross_repo_edges() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let repo_a_path = dir.path().join("repo-a");
        index_rust_crate_with_foo_bar(&repo_a_path);
        let repo_b_path = dir.path().join("repo-b");
        index_rust_crate_importing_repo_a(&repo_b_path);

        let workspace_path = dir.path().join("workspace.toml");
        std::fs::write(
            &workspace_path,
            format!(
                r#"
                    [[repo]]
                    name = "repo-a"
                    path = "{}"

                    [[repo]]
                    name = "repo-b"
                    path = "{}"
                "#,
                repo_a_path.display(),
                repo_b_path.display(),
            ),
        )
        .unwrap();

        let router = app(root, None, Some(workspace_path));
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/workspace-architecture")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["available"], true);
        assert_eq!(json["total_edges"], 1);
        let edges = json["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["from_repo"], "repo-b");
        assert_eq!(edges[0]["to_repo"], "repo-a");
        let repo_edges = json["repo_edges"].as_array().unwrap();
        assert_eq!(repo_edges.len(), 1);
        assert_eq!(repo_edges[0]["edge_count"], 1);
    }

    /// Writes a minimal real Rust crate importing `repo_b::baz::qux` --
    /// the reverse-direction counterpart to
    /// `index_rust_crate_importing_repo_a`, for building a mutual
    /// cross-repo dependency (a cycle) in tests.
    fn index_rust_crate_importing_repo_b(root: &Path) -> RepoIndex {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"repo-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let file = root.join("src/foo.rs");
        std::fs::write(&file, "use repo_b::baz::qux;\n").unwrap();
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![repowise_core::FileRecord {
                path: file,
                language: repowise_core::Language::Rust,
                lines: 1,
                symbols: vec![],
                imports: vec![repowise_core::ImportRef {
                    path: "repo_b::baz::qux".to_string(),
                    line: 1,
                    resolved_file: None,
                }],
                calls: vec![],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    #[tokio::test]
    async fn get_workspace_conformance_is_unavailable_without_a_workspace_flag() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/workspace-conformance").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["cycles"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_workspace_conformance_reports_no_cycles_for_a_dag() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let repo_a_path = dir.path().join("repo-a");
        index_rust_crate_with_foo_bar(&repo_a_path);
        let repo_b_path = dir.path().join("repo-b");
        index_rust_crate_importing_repo_a(&repo_b_path);

        let workspace_path = dir.path().join("workspace.toml");
        std::fs::write(
            &workspace_path,
            format!(
                r#"
                    [[repo]]
                    name = "repo-a"
                    path = "{}"

                    [[repo]]
                    name = "repo-b"
                    path = "{}"
                "#,
                repo_a_path.display(),
                repo_b_path.display(),
            ),
        )
        .unwrap();

        let (status, json) = {
            let router = app(root, None, Some(workspace_path));
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/api/workspace-conformance")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            )
        };

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], true);
        assert_eq!(json["cycles"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_workspace_conformance_reports_a_mutual_cross_repo_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let repo_a_path = dir.path().join("repo-a");
        index_rust_crate_importing_repo_b(&repo_a_path);
        let repo_b_path = dir.path().join("repo-b");
        index_rust_crate_importing_repo_a(&repo_b_path);

        let workspace_path = dir.path().join("workspace.toml");
        std::fs::write(
            &workspace_path,
            format!(
                r#"
                    [[repo]]
                    name = "repo-a"
                    path = "{}"

                    [[repo]]
                    name = "repo-b"
                    path = "{}"
                "#,
                repo_a_path.display(),
                repo_b_path.display(),
            ),
        )
        .unwrap();

        let router = app(root, None, Some(workspace_path));
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/workspace-conformance")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["available"], true);
        let cycles = json["cycles"].as_array().unwrap();
        assert_eq!(cycles.len(), 1);
        let mut cycle: Vec<String> = cycles[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        cycle.sort();
        assert_eq!(cycle, vec!["repo-a".to_string(), "repo-b".to_string()]);
    }

    /// Writes a file with an axum-style `.route(...)` call and a
    /// minimal hand-built index pointing at it -- `workspace_contracts`
    /// regex-scans the file's raw text directly, so the `FileRecord`'s
    /// own symbols/imports are irrelevant, consistent with this file's
    /// established "no `repowise-parser` dependency" convention.
    fn index_repo_with_producer_route(root: &Path) -> RepoIndex {
        let file = root.join("routes.rs");
        std::fs::write(
            &file,
            "router.route(\"/api/hotspots\", get(get_hotspots));\n",
        )
        .unwrap();
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![repowise_core::FileRecord {
                path: file,
                language: repowise_core::Language::Rust,
                lines: 1,
                symbols: vec![],
                imports: vec![],
                calls: vec![],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    /// Writes a file with a `fetch(...)` call and a minimal hand-built
    /// index pointing at it.
    fn index_repo_with_consumer_call(root: &Path, path: &str) -> RepoIndex {
        let file = root.join("app.js");
        std::fs::write(&file, format!("fetch(\"{path}\");\n")).unwrap();
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![repowise_core::FileRecord {
                path: file,
                language: repowise_core::Language::JavaScript,
                lines: 1,
                symbols: vec![],
                imports: vec![],
                calls: vec![],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    #[tokio::test]
    async fn get_workspace_contracts_is_unavailable_without_a_workspace_flag() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/workspace-contracts").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["matches"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_workspace_contracts_matches_a_cross_repo_producer_and_consumer() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let server_repo = dir.path().join("server");
        std::fs::create_dir_all(&server_repo).unwrap();
        index_repo_with_producer_route(&server_repo);

        let client_repo = dir.path().join("client");
        std::fs::create_dir_all(&client_repo).unwrap();
        index_repo_with_consumer_call(&client_repo, "/api/hotspots");

        let workspace_path = dir.path().join("workspace.toml");
        std::fs::write(
            &workspace_path,
            format!(
                r#"
                    [[repo]]
                    name = "server"
                    path = "{}"

                    [[repo]]
                    name = "client"
                    path = "{}"
                "#,
                server_repo.display(),
                client_repo.display(),
            ),
        )
        .unwrap();

        let router = app(root, None, Some(workspace_path));
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/workspace-contracts")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["available"], true);
        let matches = json["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["producer_repo"], "server");
        assert_eq!(matches[0]["consumer_repo"], "client");
        assert_eq!(matches[0]["path"], "/api/hotspots");
        assert_eq!(json["unmatched_consumers"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_workspace_contracts_reports_an_unmatched_consumer() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let client_repo = dir.path().join("client");
        std::fs::create_dir_all(&client_repo).unwrap();
        index_repo_with_consumer_call(&client_repo, "/api/unknown");

        let workspace_path = dir.path().join("workspace.toml");
        std::fs::write(
            &workspace_path,
            format!(
                r#"
                    [[repo]]
                    name = "client"
                    path = "{}"
                "#,
                client_repo.display(),
            ),
        )
        .unwrap();

        let router = app(root, None, Some(workspace_path));
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/workspace-contracts")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["available"], true);
        assert_eq!(json["matches"], serde_json::json!([]));
        let unmatched = json["unmatched_consumers"].as_array().unwrap();
        assert_eq!(unmatched.len(), 1);
        assert_eq!(unmatched[0]["path"], "/api/unknown");
    }

    #[tokio::test]
    async fn post_chat_tallies_reported_usage_into_api_usage() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let embeddings_response =
            r#"{"data": [{"embedding": [1.0, 0.0]}, {"embedding": [1.0, 0.0]}]}"#;
        let chat_response = r#"{"choices": [{"message": {"role": "assistant", "content": "busy() lives in busy.rs."}}], "usage": {"prompt_tokens": 40, "completion_tokens": 10, "total_tokens": 50}}"#;
        let server = ChatFixtureServer::start_sequence(vec![embeddings_response, chat_response]);
        let config = repowise_llm::LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let router = app_with_llm_config(root, Some(config));
        let (status, _json) = post_json(
            router.clone(),
            "/api/chat",
            serde_json::json!({"history": [{"role": "user", "content": "What does busy do?"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, json) = get_on(router, "/api/usage").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["chat_call_count"], 1);
        assert_eq!(json["prompt_tokens"], 40);
        assert_eq!(json["completion_tokens"], 10);
        assert_eq!(json["total_tokens"], 50);
    }

    #[tokio::test]
    async fn post_chat_falls_back_to_keyword_search_when_embeddings_fail() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        // Missing "data" -- fails to deserialize as an embeddings
        // response, simulating an endpoint that doesn't implement
        // `/v1/embeddings` at all.
        let embeddings_response = r#"{"error": "not supported"}"#;
        let chat_response = r#"{"choices": [{"message": {"role": "assistant", "content": "busy() lives in busy.rs."}}]}"#;
        let server = ChatFixtureServer::start_sequence(vec![embeddings_response, chat_response]);
        let config = repowise_llm::LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let router = app_with_llm_config(root, Some(config));
        let (status, json) = post_json(
            router,
            "/api/chat",
            serde_json::json!({"history": [{"role": "user", "content": "What does busy do?"}]}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], true);
        assert_eq!(json["reply"], "busy() lives in busy.rs.");
        assert!(server.requests()[1].contains("keyword search"));
        // The degradation is now reported, not just silently survived:
        // a keyword-backed answer and a semantic one are materially
        // different in quality and the caller is entitled to know which
        // it got.
        assert_eq!(json["retrieval_mode"], "keyword");
        assert!(
            json["retrieval_caveat"]
                .as_str()
                .expect("a degraded retrieval must carry a caveat")
                .contains("retrieval failure"),
            "{}",
            json["retrieval_caveat"]
        );
        assert_eq!(
            json["cited"][0], "busy.rs",
            "the answer's sources must come back structured, not only inside the prompt"
        );
    }

    /// Two real, parsed files -- a persisted embedding index can cover
    /// one and leave the other for top-up, which a single-file fixture
    /// can't exercise.
    fn index_with_two_files(root: &Path) -> RepoIndex {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/auth.rs"),
            "pub fn validate_token() -> bool { true }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/config.rs"),
            "pub fn load_config() -> u8 { 0 }\n",
        )
        .unwrap();
        let index = repowise_parser::build_index(root).unwrap();
        index.save(root).unwrap();
        index
    }

    /// `/api/chat` must reuse the persisted embedding index rather than
    /// re-embedding everything -- the behavior #308 added. Verified by
    /// inspecting the actual embeddings request (via `ChatFixtureServer`,
    /// which already records raw request bytes for the keyword-fallback
    /// test above), not just the response, since a wrong document count
    /// still gets a same-shaped response back.
    #[tokio::test]
    async fn post_chat_reuses_the_stored_embedding_index_and_reports_the_split() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let index = index_with_two_files(&root);
        let auth = index
            .files
            .iter()
            .find(|f| f.path.ends_with("auth.rs"))
            .unwrap();

        let mut stored = repowise_llm::EmbeddingIndex::new("embed");
        stored.entries.insert(
            repowise_llm::embedding_index::document_key(&repowise_llm::embedding_index::document(
                &root, auth,
            )),
            vec![1.0, 0.0],
        );
        stored.save(&root).unwrap();

        // Question + exactly one file (config.rs, the uncovered one),
        // so exactly two vectors come back.
        let embeddings_response =
            r#"{"data": [{"embedding": [1.0, 0.0]}, {"embedding": [0.2, 0.1]}]}"#;
        let chat_response = r#"{"choices": [{"message": {"role": "assistant", "content": "Auth checks a token."}}]}"#;
        let server = ChatFixtureServer::start_sequence(vec![embeddings_response, chat_response]);
        let config = repowise_llm::LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let router = app_with_llm_config(root.clone(), Some(config));
        let (status, json) = post_json(
            router,
            "/api/chat",
            serde_json::json!({"history": [{"role": "user", "content": "how does auth work"}]}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["retrieval_mode"], "semantic");
        assert_eq!(json["vectors_reused"], 1);
        assert_eq!(json["vectors_embedded_now"], 1);
        // No caveat: coverage is complete either way, so there is
        // nothing here for a caveat to warn about.
        assert!(json["retrieval_caveat"].is_null());

        let embed_request = &server.requests()[0];
        let config_doc = repowise_llm::embedding_index::document(
            &root,
            index
                .files
                .iter()
                .find(|f| f.path.ends_with("config.rs"))
                .unwrap(),
        );
        let auth_doc = repowise_llm::embedding_index::document(&root, auth);
        assert!(
            embed_request.contains(&config_doc.replace('\n', "\\n")),
            "config.rs has no stored vector and must be embedded: {embed_request}"
        );
        assert!(
            !embed_request.contains(&auth_doc.replace('\n', "\\n")),
            "auth.rs was already in the stored index and must not be re-embedded: {embed_request}"
        );
    }

    /// The unconfigured and keyword-fallback responses must carry no
    /// vector counts -- `null`, not `0`, since no vectors were involved
    /// at all rather than an index that covered nothing.
    #[tokio::test]
    async fn chat_responses_without_semantic_retrieval_carry_no_vector_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let router = app_with_llm_config(root, None);
        let (status, json) = post_json(
            router,
            "/api/chat",
            serde_json::json!({"history": [{"role": "user", "content": "anything"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert!(json["vectors_reused"].is_null());
        assert!(json["vectors_embedded_now"].is_null());
    }

    /// Like `get`, but drives an already-built `Router` instead of
    /// constructing a fresh `app(root, None, None)` -- needed to observe a
    /// background job's state transitions, since a fresh `app()` call
    /// would build a brand-new `AppState` (and reindex job) each time.
    async fn get_on(router: Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json = if body.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&body).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&body).into_owned())
            })
        };
        (status, json)
    }

    #[test]
    fn reindex_job_try_start_prevents_a_second_concurrent_run() {
        let job = ReindexJob::new();
        assert!(job.try_start());
        assert!(!job.try_start());
        job.finish(ReindexStatusDto::Completed {
            file_count: 1,
            other_file_count: 0,
            duration_ms: 0,
        });
        assert!(job.try_start());
    }

    #[tokio::test]
    async fn get_reindex_status_starts_idle() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/reindex").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "idle");
    }

    #[tokio::test]
    async fn post_reindex_triggers_a_background_reindex_and_reports_completion() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let router = app(root, None, None);
        let (status, json) =
            post_json(router.clone(), "/api/reindex", serde_json::Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["status"] == "running" || json["status"] == "completed");

        let mut final_json = json;
        for _ in 0..50 {
            if final_json["status"] != "running" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let (status, json) = get_on(router.clone(), "/api/reindex").await;
            assert_eq!(status, StatusCode::OK);
            final_json = json;
        }

        assert_eq!(final_json["status"], "completed");
        assert_eq!(final_json["file_count"], 1);
    }

    #[tokio::test]
    async fn post_reindex_reports_failure_when_the_root_disappears() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        std::fs::remove_dir_all(&root).unwrap();

        let router = app(root, None, None);
        let (status, _json) =
            post_json(router.clone(), "/api/reindex", serde_json::Value::Null).await;
        assert_eq!(status, StatusCode::OK);

        let mut final_json = serde_json::Value::Null;
        for _ in 0..50 {
            let (status, json) = get_on(router.clone(), "/api/reindex").await;
            assert_eq!(status, StatusCode::OK);
            if json["status"] != "running" {
                final_json = json;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert_eq!(final_json["status"], "failed");
        assert!(final_json["error"].is_string());
    }
}
