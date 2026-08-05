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
//!
//! `POST /api/webhook/github` and `POST /api/webhook/gitlab` (issue
//! #335, `ParityGaps.md`'s open-gaps list) are the third of upstream
//! repowise's five auto-sync mechanisms this port implements -- after
//! the post-commit hook (`repowise hook install`) and file-watching
//! (`repowise watch`), both CLI-only. These two need a running server
//! to receive them, which `repowise-server` didn't exist yet to provide
//! when auto-sync was first built; it does now. Both trigger the exact
//! same background-reindex job `POST /api/reindex` already exposes
//! (`trigger_reindex`, factored out so there's exactly one
//! job-triggering code path, not three that could drift), gated behind
//! `REPOWISE_WEBHOOK_SECRET`: unset, both endpoints report `503` rather
//! than silently accepting unauthenticated requests, since a webhook
//! endpoint with no verification is an open invitation to force a
//! reindex (a cheap but real denial-of-service surface) from anyone who
//! can reach the port. GitHub's `X-Hub-Signature-256` (HMAC-SHA256 over
//! the raw request body) and GitLab's `X-Gitlab-Token` (a plain shared
//! secret) are different auth shapes by design on their end, so this
//! port verifies each the way its own forge expects rather than forcing
//! one scheme onto both -- `ring::hmac`/`ring::constant_time` do the
//! actual comparison, not hand-rolled code, since a non-constant-time
//! secret comparison is exactly the kind of subtle bug a webhook secret
//! shouldn't be exposed to. No polling fallback: a workspace with no
//! forge to send it webhooks in the first place has `repowise watch` or
//! the post-commit hook already, and this port has no persisted
//! "when did I last check" state a poller would need.
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
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
    /// Sibling `.repowise-workspace/` directory for the same `--workspace
    /// <path>` this state's `workspace_repos` was resolved from (`None`
    /// under the same condition `workspace_repos` is `None`) -- computed
    /// once at startup via `repowise_workspace::workspace_state_dir`
    /// rather than re-derived per request, same "resolve once, reuse"
    /// shape as `workspace_repos` itself. Currently the only consumer is
    /// `get_workspace_contracts`'s breaking-change snapshot.
    workspace_state_dir: Arc<Option<PathBuf>>,
    reindex_job: ReindexJob,
    usage: UsageTracker,
    /// Resolved once at server startup from `REPOWISE_WEBHOOK_SECRET`
    /// (`None` if unset) -- gates `post_webhook_github`/
    /// `post_webhook_gitlab`. Read once for the same reason
    /// `llm_config` is: a pure function of its state, and tests can
    /// inject a fixture secret directly instead of racing process env
    /// vars across parallel tests.
    webhook_secret: Arc<Option<String>>,
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
    /// Which repo this describes; absent on an unscoped call and on the
    /// aggregate wrapper of a federated one (issue #337).
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    /// One entry per workspace repo, present only on `?repo=all`.
    /// Absent otherwise, so an unscoped response is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    repos: Option<Vec<OverviewDto>>,
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
            repo: None,
            repos: None,
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
    /// Which repo this describes; absent unscoped (issue #337).
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    /// One entry per workspace repo, present only on `?repo=all`.
    ///
    /// Health federates **per repo** and synthesises no workspace-wide
    /// average, matching the `get_health` MCP tool: a mean of means is
    /// not a mean, and a merged score would look authoritative while
    /// being wrong. On a federated call the flat fields describe the
    /// first repo and this list carries the real answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    repos: Option<Vec<HealthDto>>,
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
const COUPLING_LIMIT: usize = 30;

/// A JSON-serializable ranked list of `repowise_git::GitAnalytics::
/// top_co_changed_pairs`. `available: false` (with an empty list) means
/// this root has no git history to analyze -- same degrade-gracefully
/// convention as `HotspotsDto`.
#[derive(Serialize)]
struct CouplingDto {
    available: bool,
    pairs: Vec<CouplingPairDto>,
}

#[derive(Serialize)]
struct CouplingPairDto {
    file_a: String,
    file_b: String,
    /// Number of commits in the walked history that touched both files.
    count: usize,
}

/// The Architecture section's Dependencies sub-view (issue #353):
/// third-party dependencies declared across every manifest this port
/// recognizes. Declared, not resolved -- see
/// `repowise_core::deps::ExternalDependency`'s module doc.
#[derive(Serialize)]
struct ExternalDependencyDto {
    name: String,
    version: Option<String>,
    /// `"direct"`, `"dev"`, or `"build"`.
    kind: &'static str,
    /// `"cargo"`, `"npm"`, `"pypi"`, `"go"`, or `"composer"`.
    ecosystem: &'static str,
    file: String,
    line: usize,
}

/// Default rows for `GET /api/commits` -- a bounded recent window, not
/// the whole history (issue #356's own open question: `git log` on the
/// whole history is comparatively cheap on its own, but scoring every
/// listed commit eagerly would multiply `change_risk`'s real per-commit
/// diff cost by however many are listed, which is why risk is a
/// separate, on-demand `/api/commit-risk` call instead).
const COMMITS_DEFAULT_LIMIT: usize = 30;
/// Hard cap, however large a `?limit=` is requested.
const COMMITS_MAX_LIMIT: usize = 200;

#[derive(Deserialize)]
struct CommitsQuery {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Serialize)]
struct CommitDto {
    hash: String,
    /// First 7 characters of `hash`, for display.
    short_hash: String,
    author: String,
    /// The commit's subject line.
    message: String,
    /// Unix seconds (author date).
    timestamp: i64,
    files_touched: usize,
}

#[derive(Serialize)]
struct CommitsDto {
    /// `false` when `PATH` isn't a git repository -- same
    /// degrade-gracefully convention as `HotspotsDto`.
    available: bool,
    commits: Vec<CommitDto>,
}

#[derive(Deserialize)]
struct CommitRiskQuery {
    /// A single commit hash or a `base..head` range. Defaults to `HEAD`,
    /// same as `get_change_risk`.
    revspec: Option<String>,
}

#[derive(Serialize)]
struct CommitRiskDto {
    revspec: String,
    lines_added: usize,
    lines_deleted: usize,
    files_touched: usize,
    subsystems_touched: usize,
    concentration: f64,
    author: String,
    author_prior_commits: usize,
    score: f64,
}

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
    /// How much to trust this decision as recorded intent, in `[0, 1]` --
    /// derived from the source alone (an ADR file outranks a freeform
    /// README paragraph).
    confidence: f64,
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
        S::ReadmeMining {
            file,
            line,
            heading,
        } => {
            format!("readme:{}:{line} ({heading:?})", relative(root, file))
        }
        S::Inferred { file, line, model } => {
            format!("inferred:{}:{line} by {model}", relative(root, file))
        }
        S::Manual { recorded_at } => format!("manual:{recorded_at}"),
    }
}

#[derive(Serialize, Clone)]
struct SymbolDto {
    name: String,
    kind: String,
    file: String,
    start_line: usize,
    /// Added for the Knowledge Graph view's symbol-level treemap tier
    /// (issue #354): `end_line - start_line + 1` is a symbol's line
    /// span, the sizing value that level needs.
    end_line: usize,
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

#[derive(Deserialize)]
struct SemanticSearchQuery {
    q: String,
}

/// The main search box's semantic fallback (issue #357): `/api/search`
/// stays instant/substring-only (issue #63's own reasoning still
/// applies -- an embeddings call per keystroke would make instant
/// search not instant), and the frontend calls this *separately*, only
/// once `/api/search` has already come back empty for the settled
/// query. Files only, not symbols: this port's embedding index is
/// file-granularity, the same one `POST /api/chat` already reuses.
#[derive(Serialize)]
struct SemanticSearchDto {
    /// `false` when no LLM is configured, or when embeddings retrieval
    /// degraded to keyword matching internally (an unreachable/erroring
    /// endpoint) -- either way there's nothing here `/api/search`
    /// didn't already try, so it's reported as unavailable rather than
    /// as an empty-but-successful semantic result.
    available: bool,
    /// Repo-relative paths, best match first.
    files: Vec<String>,
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

/// The Architecture section's Map sub-view (issue #352): detected
/// communities within the dependency graph, sized by code volume.
#[derive(Serialize)]
struct CommunityDto {
    /// Rank by size, largest first -- stable across a call since
    /// `detect_communities` sorts deterministically.
    id: usize,
    /// Repo-relative file paths, sorted.
    files: Vec<String>,
    file_count: usize,
    total_lines: usize,
    /// Whichever language is most common among the community's files
    /// (alphabetical tiebreak for determinism).
    dominant_language: String,
}

#[derive(Serialize)]
struct CommunitiesDto {
    communities: Vec<CommunityDto>,
    /// `true` when more communities were found than `COMMUNITIES_LIMIT`
    /// and the list below was cut down to the largest ones.
    truncated: bool,
}

/// However fragmented a repo's import graph is, a module map with more
/// tiles than this stops being a map -- same reasoning as
/// `GRAPH_NODE_LIMIT`, applied to communities instead of raw files.
const COMMUNITIES_LIMIT: usize = 150;

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
    #[serde(default)]
    repo: Option<String>,
}

#[derive(Serialize)]
struct DeadCodeCandidateDto {
    /// Which repo this came from; absent on an unscoped call, so the
    /// current dashboard sees no change (issue #337).
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
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

#[derive(Deserialize)]
struct RefactorCandidatesQuery {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    repo: Option<String>,
}

#[derive(Serialize)]
struct RefactorCandidateDto {
    /// Which repo this came from; absent on an unscoped call, so the
    /// current dashboard sees no change (issue #337).
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    id: String,
    /// `break-import-cycle`, `split-god-class`, `split-by-cohesion`, or
    /// `extract-duplicate`.
    kind: String,
    title: String,
    rationale: String,
    files: Vec<String>,
    symbols: Vec<String>,
}

#[derive(Serialize)]
struct RefactorCandidatesDto {
    candidates: Vec<RefactorCandidateDto>,
    /// How many candidates matched `kind` before truncation to
    /// `REFACTOR_CANDIDATES_LIMIT` -- mirrors the `get_refactor_candidates`
    /// MCP tool's own `total_matching` field, for the same "don't
    /// silently truncate" reason.
    total_matching: usize,
}

/// Matches the `get_refactor_candidates` MCP tool's own default `limit`.
const REFACTOR_CANDIDATES_LIMIT: usize = 20;

#[derive(Deserialize)]
struct SecurityQuery {
    /// `high`, `medium`, or `low` -- everything at or above this
    /// severity. Omit for everything.
    #[serde(default)]
    min_severity: Option<String>,
    #[serde(default)]
    repo: Option<String>,
}

#[derive(Serialize)]
struct SecurityFindingDto {
    /// Which repo this came from; absent on an unscoped call, so the
    /// current dashboard sees no change (issue #337).
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    file: String,
    line: usize,
    /// `aws-access-key-id`, `private-key-block`, `github-token`,
    /// `slack-token`, or `suspicious-assignment` -- see
    /// `repowise_security::SecurityFindingKind`.
    kind: &'static str,
    severity: &'static str,
    message: String,
}

#[derive(Serialize)]
struct SecurityDto {
    findings: Vec<SecurityFindingDto>,
    /// How many findings matched `min_severity` before truncation to
    /// `SECURITY_LIMIT` -- same "don't silently truncate" convention as
    /// `RefactorCandidatesDto::total_matching`.
    total_matching: usize,
}

/// Matches the `get_security_findings` MCP tool's own default `limit`.
const SECURITY_LIMIT: usize = 100;

fn security_severity_rank(s: &str) -> Option<repowise_security::Severity> {
    match s {
        "high" => Some(repowise_security::Severity::High),
        "medium" => Some(repowise_security::Severity::Medium),
        "low" => Some(repowise_security::Severity::Low),
        _ => None,
    }
}

#[derive(Serialize)]
struct DocCoverageEntryDto {
    file: String,
    /// `"missing"` (no wiki page yet), `"fresh"` (page's embedded content
    /// hash matches the file's current content), or `"stale"` (the file
    /// changed since the page was last generated).
    status: &'static str,
}

#[derive(Serialize)]
struct DocCoverageDto {
    entries: Vec<DocCoverageEntryDto>,
    missing: usize,
    fresh: usize,
    stale: usize,
}

fn freshness_status_label(status: repowise_docs::FreshnessStatus) -> &'static str {
    match status {
        repowise_docs::FreshnessStatus::Missing => "missing",
        repowise_docs::FreshnessStatus::Fresh => "fresh",
        repowise_docs::FreshnessStatus::Stale => "stale",
    }
}

/// `GET /api/saved` (issue #358): the web-dashboard equivalent of
/// `repowise saved`. `by` mirrors the CLI's own `--by program|day` flag;
/// unlike the CLI there's no separate `--missed` mode -- the skipped-
/// command breakdown is always included as its own `missed` field, since
/// a dashboard view doesn't have the CLI's "one report or the other" flag
/// constraint.
#[derive(Deserialize)]
struct SavedQuery {
    #[serde(default)]
    by: Option<String>,
}

#[derive(Serialize)]
struct SavedGroupDto {
    /// The program name (`by=program`, the default) or `"day N"` --
    /// whole days since the epoch, matching the CLI's own bucketing --
    /// for `by=day`.
    key: String,
    runs: usize,
    saved_bytes: usize,
    approx_tokens_saved: usize,
}

#[derive(Serialize)]
struct McpToolSavingsDto {
    tool: String,
    calls: usize,
    saved_bytes: usize,
    approx_tokens_saved: usize,
}

#[derive(Serialize)]
struct MissedCommandDto {
    program: String,
    reason: String,
    count: usize,
}

#[derive(Serialize)]
struct SavedDto {
    by: String,
    /// Every field from here down is **measured**: bytes that went into
    /// a distillation and bytes that came out, for commands that
    /// actually ran (`repowise_distill::ledger::Record::is_measured`).
    distilled_runs: usize,
    raw_bytes: usize,
    kept_bytes: usize,
    saved_bytes: usize,
    approx_tokens_saved: usize,
    groups: Vec<SavedGroupDto>,
    /// From here down, **modelled**, not measured: `baseline_bytes` is
    /// the on-disk size of the files each MCP answer covered, i.e. what
    /// reading them instead would have cost -- a counterfactual, grounded
    /// in real file sizes but never summed with the measured totals
    /// above. See `repowise_distill::ledger::Record::McpResponse`.
    mcp_baseline_bytes: usize,
    mcp_response_bytes: usize,
    mcp_avoided_bytes: usize,
    mcp_approx_tokens_avoided: usize,
    mcp_tools: Vec<McpToolSavingsDto>,
    /// Calls where the actual response was bigger than the files it
    /// covered -- counted as zero avoided rather than a negative, but a
    /// real cost worth surfacing rather than silently flattering the
    /// total.
    mcp_costlier_calls: usize,
    mcp_overhead_bytes: usize,
    /// Commands the rewrite hook declined to wrap, grouped by
    /// (program, reason) -- the CLI's `--missed` report, always present
    /// here rather than gated behind a separate mode.
    missed: Vec<MissedCommandDto>,
}

fn saved_group_key(record: &repowise_distill::ledger::Record, by: &str) -> String {
    if by == "day" {
        format!("day {}", record.at / 86_400)
    } else {
        record.program.clone()
    }
}

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

/// A snapshot of this server's current configuration and indexed-repo
/// status -- the Settings view. Mostly still read-only (this port has no
/// persisted exclusion/generation config, or global server/webhook/MCP
/// config, to write to yet), except for `health_weights_toml`
/// (issue #359's first slice): the effective `HealthWeights`, serialized
/// back to the same TOML shape `--weights <FILE>` already reads, so the
/// dashboard can show and edit it without a bespoke per-field form.
#[derive(Serialize)]
struct SettingsDto {
    root: String,
    file_count: usize,
    other_file_count: usize,
    git_available: bool,
    wiki_pages_available: bool,
    llm_configured: bool,
    llm_model: Option<String>,
    health_weights_toml: String,
}

/// This port's first persisted, repo-level config file
/// (`.repowise/config.toml`, issue #359) -- everything else configurable
/// today is env-vars/CLI-flags-only. Nested under `[health_weights]`
/// rather than flat at the document root, even though it's the only
/// section so far, so a later config category (file-exclusion patterns
/// was the other candidate the issue considered, deliberately left out
/// of this first slice) can be added as a sibling table without a
/// breaking format change to existing `config.toml` files.
#[derive(Deserialize, Serialize, Default)]
struct RepoConfig {
    #[serde(default)]
    health_weights: repowise_health::HealthWeights,
}

fn repo_config_path(root: &Path) -> PathBuf {
    root.join(".repowise").join("config.toml")
}

#[derive(Deserialize)]
struct UpdateHealthWeightsDto {
    /// A full `config.toml` document, `[health_weights]` header
    /// included -- not just the bare weight keys, so the file this
    /// writes is self-describing on its own if someone opens it outside
    /// the dashboard.
    toml: String,
}

/// Never fails: a missing file is every fresh repo's normal state, and a
/// malformed one (hand-edited outside the dashboard's own validated save
/// path) degrades to defaults rather than breaking every health-scored
/// endpoint -- the same "skip what can't be read, don't error the whole
/// report" convention `repowise_distill::ledger::read` and
/// `repowise-docs`'s freshness check both already follow.
fn load_repo_config(root: &Path) -> RepoConfig {
    std::fs::read_to_string(repo_config_path(root))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

struct ApiError(anyhow::Error, StatusCode);

impl ApiError {
    /// A caller mistake, not a server fault -- naming a repo that isn't
    /// in the configured workspace, or asking for one without
    /// `--workspace`. Reported as 400 rather than the blanket 500,
    /// because a client that can't tell "you asked wrong" from "the
    /// server broke" will retry the former forever.
    fn bad_request(message: impl Into<String>) -> Self {
        ApiError(anyhow::anyhow!(message.into()), StatusCode::BAD_REQUEST)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.1, self.0.to_string()).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(err: E) -> Self {
        ApiError(err.into(), StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// A `?repo=` query parameter, shared by every federatable endpoint
/// (issue #337).
#[derive(Deserialize, Default)]
struct RepoQuery {
    #[serde(default)]
    repo: Option<String>,
}

/// One repo to serve a request from: its name (`None` for this server's
/// own root, so unscoped responses stay unlabeled) and its path.
struct RepoTarget {
    repo: Option<String>,
    root: PathBuf,
}

/// Resolve `?repo=` into the roots to answer from -- the dashboard's
/// counterpart to the MCP server's `resolve_search_targets` (issue
/// #337), with the same three meanings and the same refusals.
///
/// Omitted means this server's own root, which is the pre-#337 behaviour
/// and stays unlabeled. `all` federates across every configured
/// workspace repo. A name selects one. The latter two require
/// `--workspace`, and error rather than quietly falling back to the
/// server's own root -- that would look like an answer about the
/// workspace while describing one repo.
fn resolve_repo_targets(state: &AppState, repo: Option<&str>) -> Result<Vec<RepoTarget>, ApiError> {
    let Some(repo) = repo else {
        return Ok(vec![RepoTarget {
            repo: None,
            root: state.root.as_ref().clone(),
        }]);
    };
    let Some(repos) = state.workspace_repos.as_ref().as_ref() else {
        return Err(ApiError::bad_request(format!(
            "repo={repo:?} requires a workspace; start the server with --workspace"
        )));
    };
    if repo == "all" {
        return Ok(repos
            .iter()
            .map(|r| RepoTarget {
                repo: Some(r.name.clone()),
                root: r.path.clone(),
            })
            .collect());
    }
    match repos.iter().find(|r| r.name == repo) {
        Some(target) => Ok(vec![RepoTarget {
            repo: Some(target.name.clone()),
            root: target.path.clone(),
        }]),
        None => Err(ApiError::bad_request(format!(
            "no repo named {repo:?} in the configured workspace"
        ))),
    }
}

async fn get_overview(
    State(state): State<AppState>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<OverviewDto>, ApiError> {
    // Mirrors the MCP tool's federation exactly (issue #337): one entry
    // per repo, since counts are additive but `most_depended_on` is a
    // within-repo ranking that must not be merged across repos.
    let targets = resolve_repo_targets(&state, q.repo.as_deref())?;
    let mut per_repo = Vec::with_capacity(targets.len());
    for target in &targets {
        let index = RepoIndex::load(&target.root)?;
        let graph = repowise_graph::RepoGraph::build(&index);
        let overview = graph.overview(&index);
        let mut dto = OverviewDto::from_overview(&target.root, &overview);
        dto.repo = target.repo.clone();
        per_repo.push(dto);
    }
    if per_repo.len() == 1 {
        let mut only = per_repo.remove(0);
        // A single named repo still answers in the unscoped shape.
        only.repo = None;
        return Ok(Json(only));
    }

    // Federated: flat fields are workspace totals (all additive), and
    // `most_depended_on` is dropped because a dependent count is a
    // within-repo number -- ranking those across repos would compare
    // different scales. The per-repo entries carry it instead.
    let merge = |pick: fn(&OverviewDto) -> &Vec<(String, usize)>| {
        let mut m: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for r in &per_repo {
            for (k, v) in pick(r) {
                *m.entry(k.clone()).or_default() += v;
            }
        }
        let mut out: Vec<(String, usize)> = m.into_iter().collect();
        out.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
        out
    };
    let totals = OverviewDto {
        repo: None,
        file_count: per_repo.iter().map(|r| r.file_count).sum(),
        other_file_count: per_repo.iter().map(|r| r.other_file_count).sum(),
        by_language: merge(|r| &r.by_language),
        symbol_counts: merge(|r| &r.symbol_counts),
        total_lines: per_repo.iter().map(|r| r.total_lines).sum(),
        import_edges: per_repo.iter().map(|r| r.import_edges).sum(),
        call_edges: per_repo.iter().map(|r| r.call_edges).sum(),
        unresolved_imports: per_repo.iter().map(|r| r.unresolved_imports).sum(),
        unresolved_calls: per_repo.iter().map(|r| r.unresolved_calls).sum(),
        most_depended_on: Vec::new(),
        repos: Some(per_repo),
    };
    Ok(Json(totals))
}

async fn get_health(
    State(state): State<AppState>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<HealthDto>, ApiError> {
    let targets = resolve_repo_targets(&state, q.repo.as_deref())?;
    if targets.len() > 1 {
        let mut per_repo = Vec::with_capacity(targets.len());
        for target in &targets {
            let mut dto = health_dto_for(&target.root)?;
            dto.repo = target.repo.clone();
            per_repo.push(dto);
        }
        let mut first = health_dto_for(&targets[0].root)?;
        first.repos = Some(per_repo);
        return Ok(Json(first));
    }
    let dto = health_dto_for(&targets[0].root)?;
    Ok(Json(dto))
}

/// One repo's health report (issue #337) -- shared by the unscoped
/// path and by each entry of a federated `?repo=all` answer, so the
/// two can't compute it differently.
fn health_dto_for(root: &Path) -> Result<HealthDto, ApiError> {
    let index = RepoIndex::load(root)?;
    let graph = repowise_graph::RepoGraph::build(&index);
    // Organizational-signal markers (#313) need one `git blame` per
    // indexed file on top of a history walk -- several seconds on this
    // port's own workspace, acceptable for a full-report endpoint.
    // Degrades to skipping those six markers (not reporting zero risk)
    // when the root isn't a git repository.
    let analytics = repowise_git::GitAnalytics::collect(root).ok();
    let org_signals = analytics
        .as_ref()
        .and_then(|a| repowise_git::org_signals::collect_org_signals(root, &index, a).ok());
    let config = load_repo_config(root);
    let health = repowise_health::analyze_with_context(
        &index,
        &graph,
        &config.health_weights,
        &std::collections::HashSet::new(),
        None,
        org_signals.as_ref(),
    );

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
            file: relative(root, &f.file),
            score: f.score,
            finding_count: f.finding_count,
        })
        .collect();

    Ok(HealthDto {
        repo: None,
        repos: None,
        average_score: health.average_score,
        file_count: health.file_scores.len(),
        finding_count: health.findings.len(),
        by_kind,
        worst_files,
    })
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

/// The Commits view (issue #356): a bounded, recent-first commit list,
/// with no risk score attached -- scoring is a separate, on-demand
/// `/api/commit-risk` call per commit (see `COMMITS_DEFAULT_LIMIT`'s
/// doc comment for why).
async fn get_commits(
    State(state): State<AppState>,
    Query(query): Query<CommitsQuery>,
) -> Result<Json<CommitsDto>, ApiError> {
    let limit = query
        .limit
        .unwrap_or(COMMITS_DEFAULT_LIMIT)
        .clamp(1, COMMITS_MAX_LIMIT);
    let dto = match repowise_git::collect_recent_commits(&state.root, limit) {
        Ok(commits) => CommitsDto {
            available: true,
            commits: commits
                .into_iter()
                .map(|c| CommitDto {
                    short_hash: c.hash.chars().take(7).collect(),
                    hash: c.hash,
                    author: c.author,
                    message: c.message,
                    timestamp: c.timestamp,
                    files_touched: c.files.len(),
                })
                .collect(),
        },
        Err(_) => CommitsDto {
            available: false,
            commits: Vec::new(),
        },
    };
    Ok(Json(dto))
}

/// One commit's diff-shape risk score, computed on demand -- the same
/// `repowise_git::change_risk` the `get_change_risk` MCP tool and
/// `repowise risk` CLI command already use, exposed here so the
/// dashboard's Commits view (issue #356) can score a clicked-on commit
/// without paying for every listed commit's score up front.
async fn get_commit_risk(
    State(state): State<AppState>,
    Query(query): Query<CommitRiskQuery>,
) -> Result<Json<CommitRiskDto>, ApiError> {
    let risk = repowise_git::change_risk(&state.root, query.revspec.as_deref())?;
    Ok(Json(CommitRiskDto {
        revspec: risk.revspec,
        lines_added: risk.lines_added,
        lines_deleted: risk.lines_deleted,
        files_touched: risk.files_touched,
        subsystems_touched: risk.subsystems_touched,
        concentration: risk.concentration,
        author: risk.author,
        author_prior_commits: risk.author_prior_commits,
        score: risk.score,
    }))
}

/// Repo-wide change-coupling: the file pairs that most often change
/// together in the same commit, regardless of any import edge between
/// them -- the Architecture section's Coupling sub-view (issue #352).
/// `GitAnalytics::top_co_changed_pairs` already backs the cross-repo
/// `/api/workspace-co-changes`; this is its single-repo counterpart,
/// which had no dashboard/CLI/MCP surface at all before this endpoint.
async fn get_coupling(State(state): State<AppState>) -> Result<Json<CouplingDto>, ApiError> {
    let dto = match repowise_git::GitAnalytics::collect(&state.root) {
        Ok(analytics) => CouplingDto {
            available: true,
            pairs: analytics
                .top_co_changed_pairs(COUPLING_LIMIT)
                .into_iter()
                .map(|(a, b, count)| CouplingPairDto {
                    file_a: relative(&state.root, &a),
                    file_b: relative(&state.root, &b),
                    count,
                })
                .collect(),
        },
        Err(_) => CouplingDto {
            available: false,
            pairs: Vec::new(),
        },
    };
    Ok(Json(dto))
}

async fn get_external_deps(
    State(state): State<AppState>,
) -> Result<Json<Vec<ExternalDependencyDto>>, ApiError> {
    let deps = repowise_external_deps::collect_dependencies(&state.root)?;
    let mut deps: Vec<ExternalDependencyDto> = deps
        .into_iter()
        .map(|d| ExternalDependencyDto {
            name: d.name,
            version: d.version,
            kind: d.kind.label(),
            ecosystem: d.ecosystem,
            file: relative(&state.root, &d.file),
            line: d.line,
        })
        .collect();
    deps.sort_by(|a, b| a.file.cmp(&b.file).then(a.name.cmp(&b.name)));
    Ok(Json(deps))
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
                confidence: d.confidence,
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
            end_line: s.end_line,
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
                end_line: s.end_line,
            };
            (graph.call_in_degree(&s.id), dto)
        })
        .collect();
    symbols.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    symbols.truncate(SEARCH_LIMIT);
    let symbols: Vec<SymbolDto> = symbols.into_iter().map(|(_, dto)| dto).collect();

    Ok(Json(SearchDto { files, symbols }))
}

async fn get_search_semantic(
    State(state): State<AppState>,
    Query(query): Query<SemanticSearchQuery>,
) -> Result<Json<SemanticSearchDto>, ApiError> {
    let needle = query.q.trim().to_string();
    let Some(config) = (!needle.is_empty())
        .then(|| state.llm_config.as_ref().clone())
        .flatten()
    else {
        return Ok(Json(SemanticSearchDto {
            available: false,
            files: Vec::new(),
        }));
    };

    let index = RepoIndex::load(&state.root)?;
    let root = (*state.root).clone();
    let retrieval = tokio::task::spawn_blocking(move || {
        repowise_llm::retrieve(&root, &index, &needle, &config)
    })
    .await
    .map_err(anyhow::Error::from)?;

    if retrieval.mode != repowise_llm::RetrievalMode::Semantic {
        return Ok(Json(SemanticSearchDto {
            available: false,
            files: Vec::new(),
        }));
    }

    Ok(Json(SemanticSearchDto {
        available: true,
        files: retrieval.cited,
    }))
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

/// A coarser, module-level view of the same import graph `/api/graph`
/// exposes at file granularity -- a bolted-on toggle over existing
/// data, not upstream's full continuous-zoom Knowledge Graph canvas
/// (issue #354: reading upstream's own docs found the marginal *fact*
/// a full repo→module→file→symbol zoom would expose is thin relative
/// to the custom camera/culling renderer it would take to build one;
/// this covers the one genuinely missing layer -- module grouping --
/// cheaply instead). A "module" is a file's parent directory,
/// repo-relative -- generic across any repo layout, unlike guessing at
/// language-specific package conventions.
///
/// Returns the exact same shape as `/api/graph` (`GraphDto`) so the
/// frontend can reuse its layout/rendering code unchanged; a module
/// node's `language` is whichever language is most common among its
/// files (ties broken alphabetically for determinism).
async fn get_graph_modules(State(state): State<AppState>) -> Result<Json<GraphDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let graph = repowise_graph::RepoGraph::build(&index);

    let module_of = |path: &Path| -> String {
        let rel = relative(&state.root, path);
        match rel.rfind('/') {
            Some(i) => rel[..i].to_string(),
            None => ".".to_string(),
        }
    };

    let mut languages_by_module: std::collections::HashMap<
        String,
        std::collections::HashMap<String, usize>,
    > = std::collections::HashMap::new();
    for file in &index.files {
        *languages_by_module
            .entry(module_of(&file.path))
            .or_default()
            .entry(file.language.label().to_string())
            .or_insert(0) += 1;
    }

    let mut module_edges: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for file in &index.files {
        let from_module = module_of(&file.path);
        for dep in graph.dependencies_of(&file.path) {
            let to_module = module_of(&dep);
            if to_module != from_module {
                module_edges.insert((from_module.clone(), to_module));
            }
        }
    }

    let mut degree: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (from, to) in &module_edges {
        *degree.entry(from.as_str()).or_insert(0) += 1;
        *degree.entry(to.as_str()).or_insert(0) += 1;
    }

    let mut modules: Vec<&String> = languages_by_module.keys().collect();
    modules.sort_by(|a, b| {
        degree
            .get(a.as_str())
            .copied()
            .unwrap_or(0)
            .cmp(&degree.get(b.as_str()).copied().unwrap_or(0))
            .reverse()
            .then_with(|| a.cmp(b))
    });
    let truncated = modules.len() > GRAPH_NODE_LIMIT;
    modules.truncate(GRAPH_NODE_LIMIT);
    let included: std::collections::HashSet<&str> = modules.iter().map(|m| m.as_str()).collect();

    let nodes = modules
        .iter()
        .map(|module| {
            let language = languages_by_module[*module]
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(lang, _)| lang.clone())
                .unwrap_or_default();
            GraphNodeDto {
                id: (*module).clone(),
                language,
            }
        })
        .collect();

    let edges = module_edges
        .into_iter()
        .filter(|(from, to)| included.contains(from.as_str()) && included.contains(to.as_str()))
        .map(|(from, to)| GraphEdgeDto { from, to })
        .collect();

    Ok(Json(GraphDto {
        nodes,
        edges,
        truncated,
    }))
}

/// The Architecture section's Map sub-view (issue #352): Louvain
/// modularity-based community detection over the file-level import
/// graph, sized by code volume -- upstream's own words, per its
/// `docs/start/DASHBOARD.md`: "the detected communities within the
/// dependency graph laid out on a module map, with sizing proportional
/// to code volume in each component." See
/// `repowise_graph::community`'s module doc for the algorithm and why
/// it's the right read of that description.
async fn get_communities(State(state): State<AppState>) -> Result<Json<CommunitiesDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let graph = repowise_graph::RepoGraph::build(&index);

    let nodes: Vec<PathBuf> = index.files.iter().map(|f| f.path.clone()).collect();
    let mut edges: Vec<(PathBuf, PathBuf)> = Vec::new();
    for file in &index.files {
        for dep in graph.dependencies_of(&file.path) {
            edges.push((file.path.clone(), dep));
        }
    }

    let communities = repowise_graph::detect_communities(&nodes, &edges);
    let truncated = communities.len() > COMMUNITIES_LIMIT;

    let lines_of: std::collections::HashMap<&Path, usize> = index
        .files
        .iter()
        .map(|f| (f.path.as_path(), f.lines))
        .collect();
    let language_of: std::collections::HashMap<&Path, &str> = index
        .files
        .iter()
        .map(|f| (f.path.as_path(), f.language.label()))
        .collect();

    let communities = communities
        .into_iter()
        .take(COMMUNITIES_LIMIT)
        .enumerate()
        .map(|(id, files)| {
            let total_lines: usize = files
                .iter()
                .map(|f| lines_of.get(f.as_path()).copied().unwrap_or(0))
                .sum();
            let mut language_counts: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for f in &files {
                if let Some(&language) = language_of.get(f.as_path()) {
                    *language_counts.entry(language).or_insert(0) += 1;
                }
            }
            let dominant_language = language_counts
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(language, _)| language.to_string())
                .unwrap_or_default();
            CommunityDto {
                id,
                file_count: files.len(),
                total_lines,
                dominant_language,
                files: files.iter().map(|f| relative(&state.root, f)).collect(),
            }
        })
        .collect();

    Ok(Json(CommunitiesDto {
        communities,
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
    let config = load_repo_config(&state.root);
    let report = repowise_health::analyze_with_weights(&index, &graph, &config.health_weights);

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
    let targets = resolve_repo_targets(&state, query.repo.as_deref())?;

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

    // total_matching sums across repos before the shared cap truncates,
    // so a capped federated answer still reports the true total rather
    // than one repo's share of it.
    let mut all: Vec<DeadCodeCandidateDto> = Vec::new();
    let mut total_matching = 0usize;
    for target in &targets {
        let index = RepoIndex::load(&target.root)?;
        let graph = repowise_graph::RepoGraph::build(&index);
        let matching: Vec<_> = repowise_health::find_dead_code(&index, &graph)
            .into_iter()
            .filter(|c| c.confidence >= threshold)
            .collect();
        total_matching += matching.len();
        all.extend(matching.into_iter().map(|c| DeadCodeCandidateDto {
            repo: target.repo.clone(),
            file: relative(&target.root, &c.file),
            symbol: c.symbol,
            line: c.line,
            confidence: c.confidence.label().to_string(),
            risk_factors: c.risk_factors,
        }));
    }
    all.truncate(DEAD_CODE_LIMIT);

    Ok(Json(DeadCodeDto {
        candidates: all,
        total_matching,
    }))
}

async fn get_refactor_candidates(
    State(state): State<AppState>,
    Query(query): Query<RefactorCandidatesQuery>,
) -> Result<Json<RefactorCandidatesDto>, ApiError> {
    let targets = resolve_repo_targets(&state, query.repo.as_deref())?;

    if let Some(kind) = query.kind.as_deref() {
        match kind {
            "break-import-cycle" | "split-god-class" | "split-by-cohesion"
            | "extract-duplicate" => {}
            other => {
                return Err(anyhow::anyhow!(
                    "kind must be break-import-cycle/split-god-class/split-by-cohesion/\
                     extract-duplicate, got {other:?}"
                )
                .into());
            }
        }
    }

    let mut all: Vec<RefactorCandidateDto> = Vec::new();
    let mut total_matching = 0usize;
    for target in &targets {
        let index = RepoIndex::load(&target.root)?;
        let graph = repowise_graph::RepoGraph::build(&index);
        let mut found = repowise_refactor::find_refactor_candidates(&index, &graph);
        if let Some(kind) = query.kind.as_deref() {
            found.retain(|c| c.kind.label() == kind);
        }
        total_matching += found.len();
        all.extend(found.into_iter().map(|c| RefactorCandidateDto {
            repo: target.repo.clone(),
            id: c.id,
            kind: c.kind.label().to_string(),
            title: c.title,
            rationale: c.rationale,
            files: c.files,
            symbols: c.symbols,
        }));
    }
    all.truncate(REFACTOR_CANDIDATES_LIMIT);

    Ok(Json(RefactorCandidatesDto {
        candidates: all,
        total_matching,
    }))
}

async fn get_security(
    State(state): State<AppState>,
    Query(query): Query<SecurityQuery>,
) -> Result<Json<SecurityDto>, ApiError> {
    let targets = resolve_repo_targets(&state, query.repo.as_deref())?;

    let min_rank = match query.min_severity.as_deref() {
        Some(min) => match security_severity_rank(min) {
            Some(rank) => Some(rank),
            None => {
                return Err(
                    anyhow::anyhow!("min_severity must be high/medium/low, got {min:?}").into(),
                );
            }
        },
        None => None,
    };

    let mut all: Vec<SecurityFindingDto> = Vec::new();
    let mut total_matching = 0usize;
    for target in &targets {
        let index = RepoIndex::load(&target.root)?;
        let mut found = repowise_security::scan(&index);
        if let Some(rank) = min_rank {
            found.retain(|f| f.severity >= rank);
        }
        total_matching += found.len();
        all.extend(found.into_iter().map(|f| SecurityFindingDto {
            repo: target.repo.clone(),
            file: relative(&target.root, &f.file),
            line: f.line,
            kind: f.kind.label(),
            severity: f.severity.label(),
            message: f.message,
        }));
    }
    all.truncate(SECURITY_LIMIT);

    Ok(Json(SecurityDto {
        findings: all,
        total_matching,
    }))
}

async fn get_doc_coverage(State(state): State<AppState>) -> Result<Json<DocCoverageDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let report = repowise_docs::check_freshness(&index);
    let (missing, fresh, stale) = report.counts();

    let entries = report
        .entries
        .into_iter()
        .map(|e| DocCoverageEntryDto {
            file: relative(&state.root, &e.file),
            status: freshness_status_label(e.status),
        })
        .collect();

    Ok(Json(DocCoverageDto {
        entries,
        missing,
        fresh,
        stale,
    }))
}

async fn get_saved(
    State(state): State<AppState>,
    Query(query): Query<SavedQuery>,
) -> Result<Json<SavedDto>, ApiError> {
    use repowise_distill::ledger::{approx_tokens, Kind};
    use std::collections::BTreeMap;

    let by = query.by.as_deref().unwrap_or("program");
    if by != "program" && by != "day" {
        return Err(anyhow::anyhow!("by must be `program` or `day`, got {by:?}").into());
    }

    let store_dir = repowise_distill::store::store_dir(&state.root, None);
    let records = repowise_distill::ledger::read(&store_dir);

    let distilled: Vec<_> = records
        .iter()
        .filter(|r| r.kind == Kind::Distilled)
        .collect();
    let raw_bytes: usize = distilled.iter().map(|r| r.raw_bytes).sum();
    let kept_bytes: usize = distilled.iter().map(|r| r.kept_bytes).sum();
    let saved_bytes: usize = distilled.iter().map(|r| r.saved_bytes()).sum();

    let mut group_totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in &distilled {
        let entry = group_totals.entry(saved_group_key(r, by)).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.saved_bytes();
    }
    let groups = group_totals
        .into_iter()
        .map(|(key, (runs, bytes))| SavedGroupDto {
            key,
            runs,
            saved_bytes: bytes,
            approx_tokens_saved: approx_tokens(bytes),
        })
        .collect();

    let mcp: Vec<_> = records
        .iter()
        .filter(|r| r.kind == Kind::McpResponse)
        .collect();
    let mcp_baseline_bytes: usize = mcp.iter().map(|r| r.raw_bytes).sum();
    let mcp_response_bytes: usize = mcp.iter().map(|r| r.kept_bytes).sum();
    let mcp_avoided_bytes: usize = mcp.iter().map(|r| r.saved_bytes()).sum();

    let mut mcp_totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in &mcp {
        let entry = mcp_totals.entry(r.program.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.saved_bytes();
    }
    let mcp_tools = mcp_totals
        .into_iter()
        .map(|(tool, (calls, bytes))| McpToolSavingsDto {
            tool,
            calls,
            saved_bytes: bytes,
            approx_tokens_saved: approx_tokens(bytes),
        })
        .collect();

    let costlier: Vec<_> = mcp.iter().filter(|r| r.kept_bytes > r.raw_bytes).collect();
    let mcp_costlier_calls = costlier.len();
    let mcp_overhead_bytes: usize = costlier
        .iter()
        .map(|r| r.kept_bytes.saturating_sub(r.raw_bytes))
        .sum();

    let mut missed_totals: BTreeMap<(String, String), usize> = BTreeMap::new();
    for r in records.iter().filter(|r| r.kind == Kind::Skipped) {
        *missed_totals
            .entry((r.program.clone(), r.detail.clone()))
            .or_insert(0) += 1;
    }
    let mut missed: Vec<_> = missed_totals
        .into_iter()
        .map(|((program, reason), count)| MissedCommandDto {
            program,
            reason,
            count,
        })
        .collect();
    missed.sort_by_key(|m| std::cmp::Reverse(m.count));

    Ok(Json(SavedDto {
        by: by.to_string(),
        distilled_runs: distilled.len(),
        raw_bytes,
        kept_bytes,
        saved_bytes,
        approx_tokens_saved: approx_tokens(saved_bytes),
        groups,
        mcp_baseline_bytes,
        mcp_response_bytes,
        mcp_avoided_bytes,
        mcp_approx_tokens_avoided: approx_tokens(mcp_avoided_bytes),
        mcp_tools,
        mcp_costlier_calls,
        mcp_overhead_bytes,
        missed,
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

/// Real cross-repo import resolution across every workspace repo (Rust,
/// Python, Java, Kotlin, Scala, Go, C#, PHP -- see
/// `repowise_graph::cross_repo::MODULE_MAP_LANGUAGES`) -- which repos
/// depend on which others, and the individual import sites behind each
/// dependency. `edges` is capped at
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
struct BrokenContractDto {
    path: String,
    consumer_repo: String,
    consumer_file: String,
    previous_producer_repo: String,
    /// `None` when the consumer call site itself is gone rather than
    /// merely unmatched -- see `repowise_workspace::BrokenContract`'s
    /// own doc comment.
    reason: Option<&'static str>,
}

impl From<repowise_workspace::BrokenContract> for BrokenContractDto {
    fn from(b: repowise_workspace::BrokenContract) -> Self {
        BrokenContractDto {
            path: b.key.path,
            consumer_repo: b.key.consumer_repo,
            consumer_file: b.key.consumer_file.display().to_string(),
            previous_producer_repo: b.key.producer_repo,
            reason: b.reason.map(|r| r.label()),
        }
    }
}

#[derive(Serialize)]
struct WorkspaceContractsDto {
    available: bool,
    matches: Vec<ContractMatchDto>,
    unmatched_consumers: Vec<UnmatchedConsumerDto>,
    /// Contracts that resolved in the last call to this endpoint (or the
    /// last `repowise workspace-contracts` CLI run against the same
    /// workspace file -- they share one snapshot) and don't anymore. See
    /// `repowise_workspace::workspace_contract_changes`'s own doc
    /// comment: every call both reads and overwrites the snapshot, so
    /// polling this endpoint on a schedule is itself how it stays
    /// current.
    broken: Vec<BrokenContractDto>,
}

/// Regex-based HTTP producer/consumer route matching across every
/// workspace repo -- see `repowise_workspace::workspace_contracts`'s
/// own doc comment for why this is coarse and heuristic by design
/// (no cross-repo symbol resolution involved, just a fixed pattern
/// table over raw source text). `available: false` (empty lists) when
/// no workspace was configured, same shape as every other workspace
/// endpoint.
async fn get_workspace_contracts(State(state): State<AppState>) -> Json<WorkspaceContractsDto> {
    let dto = match (
        state.workspace_repos.as_ref(),
        state.workspace_state_dir.as_ref(),
    ) {
        (Some(repos), Some(state_dir)) => {
            let (report, broken) = repowise_workspace::workspace_contract_changes(repos, state_dir);
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
                broken: broken.into_iter().map(BrokenContractDto::from).collect(),
            }
        }
        _ => WorkspaceContractsDto {
            available: false,
            matches: Vec::new(),
            unmatched_consumers: Vec::new(),
            broken: Vec::new(),
        },
    };
    Json(dto)
}

async fn get_settings(State(state): State<AppState>) -> Result<Json<SettingsDto>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let git_available = repowise_git::GitAnalytics::collect(&state.root).is_ok();
    let wiki_pages_available = !wiki_indexed_files(&state.root, &index).is_empty();
    let llm_config = state.llm_config.as_ref().clone();
    let config = load_repo_config(&state.root);

    Ok(Json(SettingsDto {
        root: state.root.display().to_string(),
        file_count: index.files.len(),
        other_file_count: index.other_files,
        git_available,
        wiki_pages_available,
        llm_configured: llm_config.is_some(),
        llm_model: llm_config.map(|c| c.model),
        health_weights_toml: toml::to_string_pretty(&config)
            .unwrap_or_else(|_| "[health_weights]\n".to_string()),
    }))
}

/// The write half of issue #359's first slice: validates `body.toml`
/// parses as a `RepoConfig` (the same `[health_weights]`-nested shape
/// `GET /api/settings`'s own `health_weights_toml` field renders), then
/// persists it verbatim to `.repowise/config.toml` -- the user's own
/// formatting/comments/ordering survive a round trip, since this stores
/// the submitted text directly rather than a re-serialized copy.
/// Malformed input is reported as a normal `ApiError`, matching every
/// other invalid-input case in this module (`get_refactor_candidates`'s
/// `kind`, `get_saved`'s `by`) rather than a distinct status code.
async fn post_settings_health_weights(
    State(state): State<AppState>,
    Json(body): Json<UpdateHealthWeightsDto>,
) -> Result<Json<SettingsDto>, ApiError> {
    let _: RepoConfig = toml::from_str(&body.toml)
        .map_err(|e| anyhow::anyhow!("not a valid config.toml document: {e}"))?;

    let path = repo_config_path(&state.root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &body.toml)?;

    get_settings(State(state)).await
}

/// Kick off a background reindex (`repowise_parser::build_index`, the
/// same implementation `repowise-cli`'s `init`/`update` commands use) if
/// one isn't already running, and return the job's current status.
/// Never errors on a bad root -- a reindex failure surfaces as a
/// `Failed` status for the dashboard to render, not a 500. The one
/// job-triggering code path shared by `POST /api/reindex` and the two
/// webhook endpoints, so all three can't drift out of sync with each
/// other.
fn trigger_reindex(state: &AppState) -> ReindexStatusDto {
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
    state.reindex_job.snapshot()
}

async fn post_reindex(State(state): State<AppState>) -> Json<ReindexStatusDto> {
    Json(trigger_reindex(&state))
}

/// The dashboard polls this to render the live job banner.
async fn get_reindex_status(State(state): State<AppState>) -> Json<ReindexStatusDto> {
    Json(state.reindex_job.snapshot())
}

/// Hex-decodes a lowercase- or uppercase-hex string into raw bytes.
/// `None` on odd length or a non-hex-digit character -- a small,
/// self-contained decoder rather than a new dependency for the one
/// place this port needs it (GitHub's `X-Hub-Signature-256` header),
/// the same "small enough to write directly" call this port already
/// made for the web frontend's own six-character percent-encoder.
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Verifies a GitHub webhook's `X-Hub-Signature-256: sha256=<hex>`
/// header: HMAC-SHA256 of the raw request body, keyed on the shared
/// secret. `ring::hmac::verify` does the actual comparison in constant
/// time -- a non-constant-time comparison here would leak the correct
/// signature one byte at a time to a patient attacker, exactly the kind
/// of subtle bug a webhook secret shouldn't be exposed to.
fn verify_github_signature(secret: &str, body: &[u8], signature_header: &str) -> bool {
    let Some(hex_sig) = signature_header.strip_prefix("sha256=") else {
        return false;
    };
    let Some(sig_bytes) = decode_hex(hex_sig) else {
        return false;
    };
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    ring::hmac::verify(&key, body, &sig_bytes).is_ok()
}

/// Verifies a GitLab webhook's `X-Gitlab-Token` header: a plain shared
/// secret, compared directly rather than HMAC'd (GitLab's own scheme,
/// not this port's choice). `ring::constant_time::verify_slices_are_equal`
/// covers `verify_github_signature`'s HMAC-tag comparison but is
/// explicitly deprecated for exactly this kind of direct external use
/// ("no promises regarding side channels"), so this is a small,
/// self-contained constant-time comparison instead: XOR every
/// same-length byte pair and OR the differences together, so the number
/// of loop iterations (the only thing an attacker could time) never
/// depends on *where* a mismatch is, only on the (public) secret
/// length. A length mismatch returns `false` immediately -- lengths
/// aren't secret, so there's no timing information to protect there.
fn verify_gitlab_token(secret: &str, token_header: &str) -> bool {
    let (a, b) = (secret.as_bytes(), token_header.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `503` (not `404`, since the route genuinely exists -- it's just
/// unusable without configuration) when `REPOWISE_WEBHOOK_SECRET` isn't
/// set, and `401` on a missing or invalid signature/token -- distinct
/// statuses because they mean different things to whoever's debugging a
/// misconfigured webhook: one says "this server isn't set up for
/// webhooks at all", the other says "your forge and this server
/// disagree about the secret".
enum WebhookError {
    NotConfigured,
    Unauthorized,
}

impl IntoResponse for WebhookError {
    fn into_response(self) -> Response {
        match self {
            WebhookError::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "REPOWISE_WEBHOOK_SECRET is not set on this server",
            )
                .into_response(),
            WebhookError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "invalid webhook signature").into_response()
            }
        }
    }
}

/// GitHub webhook receiver: any event triggers a reindex (this port has
/// no per-event-type filtering -- a push, a merge, a branch update all
/// mean "the tree may have changed", and a reindex is cheap enough not
/// to need finer-grained triggering). See this module's own doc comment
/// for the auth model.
async fn post_webhook_github(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ReindexStatusDto>, WebhookError> {
    let secret = state
        .webhook_secret
        .as_ref()
        .as_ref()
        .ok_or(WebhookError::NotConfigured)?;
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .ok_or(WebhookError::Unauthorized)?;
    if !verify_github_signature(secret, &body, signature) {
        return Err(WebhookError::Unauthorized);
    }
    Ok(Json(trigger_reindex(&state)))
}

/// GitLab webhook receiver -- see `post_webhook_github`'s own doc
/// comment for the shared "any event reindexes" reasoning, and this
/// module's doc comment for the auth model.
async fn post_webhook_gitlab(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ReindexStatusDto>, WebhookError> {
    let secret = state
        .webhook_secret
        .as_ref()
        .as_ref()
        .ok_or(WebhookError::NotConfigured)?;
    let token = headers
        .get("x-gitlab-token")
        .and_then(|v| v.to_str().ok())
        .ok_or(WebhookError::Unauthorized)?;
    if !verify_gitlab_token(secret, token) {
        return Err(WebhookError::Unauthorized);
    }
    Ok(Json(trigger_reindex(&state)))
}

/// Build the axum `Router` — separated from `serve` so tests can drive
/// requests directly against it (via `tower::ServiceExt::oneshot`)
/// without binding a real socket. `static_dir`, if given, serves the
/// built `repowise-web` frontend (e.g. `crates/repowise-web/dist` after
/// `trunk build`) as a fallback for any path the JSON API doesn't claim.
pub fn app(root: PathBuf, static_dir: Option<PathBuf>, workspace: Option<PathBuf>) -> Router {
    let workspace_state_dir = workspace
        .as_deref()
        .map(repowise_workspace::workspace_state_dir);
    let workspace_repos = workspace.and_then(|path| repowise_workspace::load_resolved(&path).ok());
    let state = AppState {
        root: Arc::new(root),
        llm_config: Arc::new(repowise_llm::LlmConfig::from_env()),
        workspace_repos: Arc::new(workspace_repos),
        workspace_state_dir: Arc::new(workspace_state_dir),
        reindex_job: ReindexJob::new(),
        usage: UsageTracker::new(),
        webhook_secret: Arc::new(std::env::var("REPOWISE_WEBHOOK_SECRET").ok()),
    };
    build_router(state, static_dir)
}

fn build_router(state: AppState, static_dir: Option<PathBuf>) -> Router {
    let router = Router::new()
        .route("/api/overview", get(get_overview))
        .route("/api/health", get(get_health))
        .route("/api/hotspots", get(get_hotspots))
        .route("/api/commits", get(get_commits))
        .route("/api/commit-risk", get(get_commit_risk))
        .route("/api/coupling", get(get_coupling))
        .route("/api/external-deps", get(get_external_deps))
        .route("/api/decisions", get(get_decisions))
        .route("/api/symbols", get(get_symbols))
        .route("/api/wiki-pages", get(get_wiki_pages))
        .route("/api/wiki", get(get_wiki))
        .route("/api/search", get(get_search))
        .route("/api/search-semantic", get(get_search_semantic))
        .route("/api/graph", get(get_graph))
        .route("/api/graph-modules", get(get_graph_modules))
        .route("/api/communities", get(get_communities))
        .route("/api/ownership", get(get_ownership))
        .route("/api/symbol", get(get_symbol_detail))
        .route("/api/decision", get(get_decision_detail))
        .route("/api/stats", get(get_stats))
        .route("/api/files", get(get_files))
        .route("/api/contributors", get(get_contributors))
        .route("/api/coverage", get(get_coverage))
        .route("/api/dead-code", get(get_dead_code))
        .route("/api/refactor-candidates", get(get_refactor_candidates))
        .route("/api/security", get(get_security))
        .route("/api/doc-coverage", get(get_doc_coverage))
        .route("/api/saved", get(get_saved))
        .route("/api/chat", post(post_chat))
        .route("/api/reindex", get(get_reindex_status).post(post_reindex))
        .route("/api/webhook/github", post(post_webhook_github))
        .route("/api/webhook/gitlab", post(post_webhook_gitlab))
        .route("/api/settings", get(get_settings))
        .route(
            "/api/settings/health-weights",
            post(post_settings_health_weights),
        )
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

    /// Two indexed repos plus a workspace TOML naming both, so
    /// `?repo=` has something real to resolve against.
    fn workspace_of_two(dir: &Path) -> (PathBuf, PathBuf) {
        for (name, n) in [("repo-a", 1usize), ("repo-b", 2usize)] {
            let path = dir.join(name);
            std::fs::create_dir_all(path.join("src")).unwrap();
            std::fs::write(
                path.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
            )
            .unwrap();
            for i in 0..n {
                std::fs::write(
                    path.join("src").join(format!("f{i}.rs")),
                    format!("pub fn f{i}() -> i32 {{ {i} }}\n"),
                )
                .unwrap();
            }
            index_dir_at(&path);
        }
        let ws = dir.join("ws.toml");
        std::fs::write(
            &ws,
            "[[repo]]\nname = \"repo-a\"\npath = \"repo-a\"\n\n             [[repo]]\nname = \"repo-b\"\npath = \"repo-b\"\n",
        )
        .unwrap();
        (dir.join("repo-a"), ws)
    }

    fn index_dir_at(root: &Path) {
        let discovered = repowise_core::discover_files(root).unwrap();
        let mut files = Vec::new();
        let mut other_files = 0;
        for entry in discovered {
            if matches!(entry.language, repowise_core::Language::Other) {
                other_files += 1;
                continue;
            }
            let source = std::fs::read_to_string(&entry.path).unwrap();
            match repowise_parser::parse_file(&entry.path, entry.language, &source).unwrap() {
                Some(record) => files.push(record),
                None => other_files += 1,
            }
        }
        RepoIndex {
            root: root.to_path_buf(),
            files,
            other_files,
            indexed_commit: None,
        }
        .save(root)
        .unwrap();
    }

    async fn get_ws(
        root: PathBuf,
        workspace: PathBuf,
        uri: &str,
    ) -> (StatusCode, serde_json::Value) {
        let response = app(root, None, Some(workspace))
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
            serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null)
        };
        (status, json)
    }

    /// `?repo=all` federates, and the flat counts are the workspace
    /// total -- checked against the breakdown, not a literal, so the two
    /// can't drift apart.
    #[tokio::test]
    async fn overview_federates_across_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (own, ws) = workspace_of_two(&root);

        let (status, json) = get_ws(own, ws, "/api/overview?repo=all").await;
        assert_eq!(status, StatusCode::OK);

        let repos = json["repos"]
            .as_array()
            .expect("federated call lists repos");
        assert_eq!(repos.len(), 2);
        let sum: u64 = repos
            .iter()
            .map(|r| r["file_count"].as_u64().unwrap())
            .sum();
        assert_eq!(
            json["file_count"].as_u64().unwrap(),
            sum,
            "the total must equal the breakdown it claims to total"
        );
        assert_eq!(json["file_count"].as_u64().unwrap(), 3, "1 + 2 files");
        assert!(
            json["most_depended_on"].as_array().unwrap().is_empty(),
            "within-repo dependent counts must not be ranked across repos"
        );
    }

    /// An unscoped call keeps its exact pre-#337 shape.
    #[tokio::test]
    async fn an_unscoped_overview_omits_the_repos_field() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (own, ws) = workspace_of_two(&root);

        let (status, json) = get_ws(own, ws, "/api/overview").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            json.get("repos").is_none(),
            "unscoped must not gain a field"
        );
        assert_eq!(json["file_count"].as_u64().unwrap(), 1, "own repo only");
    }

    /// Naming a repo without --workspace is a client error, not a 500:
    /// a client that can't tell "you asked wrong" from "the server
    /// broke" will retry the former forever.
    #[tokio::test]
    async fn overview_with_a_repo_but_no_workspace_is_a_400() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_dir_at(&root);

        let (status, _) = get(root, "/api/overview?repo=all").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    /// Every federatable endpoint honours `?repo=all`, not just the one
    /// that shipped first -- and each labels its results.
    #[tokio::test]
    async fn every_federatable_endpoint_accepts_repo_all() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (own, ws) = workspace_of_two(&root);

        for uri in [
            "/api/health?repo=all",
            "/api/dead-code?repo=all",
            "/api/refactor-candidates?repo=all",
            "/api/security?repo=all",
        ] {
            let (status, _) = get_ws(own.clone(), ws.clone(), uri).await;
            assert_eq!(status, StatusCode::OK, "{uri} should federate");
        }
    }

    /// Health federates per repo and synthesises no merged average --
    /// a mean of means is not a mean.
    #[tokio::test]
    async fn health_federates_per_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (own, ws) = workspace_of_two(&root);

        let (status, json) = get_ws(own, ws, "/api/health?repo=all").await;
        assert_eq!(status, StatusCode::OK);
        let repos = json["repos"]
            .as_array()
            .expect("federated health lists repos");
        assert_eq!(repos.len(), 2);
        assert!(
            repos.iter().all(|r| r["repo"].is_string()),
            "every entry must name its repo"
        );
    }

    /// Dead-code results carry the repo they came from, and the total
    /// sums across repos rather than reporting one repo's share.
    #[tokio::test]
    async fn dead_code_federates_and_labels_each_result() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (own, ws) = workspace_of_two(&root);

        let (status, json) = get_ws(own, ws, "/api/dead-code?repo=all").await;
        assert_eq!(status, StatusCode::OK);
        let candidates = json["candidates"].as_array().unwrap();
        assert!(!candidates.is_empty(), "fixture must produce candidates");
        let repos: std::collections::HashSet<&str> = candidates
            .iter()
            .filter_map(|c| c["repo"].as_str())
            .collect();
        assert!(
            repos.contains("repo-a") && repos.contains("repo-b"),
            "a federated call must reach both repos, got {repos:?}"
        );
        assert_eq!(
            json["total_matching"].as_u64().unwrap() as usize,
            candidates.len(),
            "nothing was truncated here, so the total must match the list"
        );
    }

    /// Unscoped calls keep their exact pre-#337 shape on every endpoint.
    #[tokio::test]
    async fn unscoped_calls_carry_no_repo_labels() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (own, ws) = workspace_of_two(&root);

        let (_, health) = get_ws(own.clone(), ws.clone(), "/api/health").await;
        assert!(health.get("repos").is_none());
        assert!(health.get("repo").is_none());

        let (_, dead) = get_ws(own, ws, "/api/dead-code").await;
        for c in dead["candidates"].as_array().unwrap() {
            assert!(
                c.get("repo").is_none(),
                "unscoped results must stay unlabeled"
            );
        }
    }

    /// An unknown repo name is likewise the caller's mistake.
    #[tokio::test]
    async fn overview_with_an_unknown_repo_is_a_400() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (own, ws) = workspace_of_two(&root);

        let (status, _) = get_ws(own, ws, "/api/overview?repo=nope").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
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

    /// Two files in different directories, `mod_a/a.rs` importing
    /// `mod_b/b.rs` -- enough to exercise `/api/graph-modules`'
    /// directory-level aggregation across a module boundary.
    fn index_with_a_cross_module_import(root: &Path) -> RepoIndex {
        std::fs::create_dir_all(root.join("mod_a")).unwrap();
        std::fs::create_dir_all(root.join("mod_b")).unwrap();
        let a = root.join("mod_a/a.rs");
        let b = root.join("mod_b/b.rs");
        std::fs::write(&a, "mod b;\n").unwrap();
        std::fs::write(&b, "pub fn helper() {}\n").unwrap();
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![
                repowise_core::FileRecord {
                    path: a,
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

    #[tokio::test]
    async fn get_graph_modules_aggregates_files_up_to_their_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_a_cross_module_import(&root);

        let (status, json) = get(root, "/api/graph-modules").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["truncated"], false);
        let nodes: Vec<&str> = json["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert_eq!(nodes.len(), 2, "{nodes:?}");
        assert!(nodes.contains(&"mod_a"));
        assert!(nodes.contains(&"mod_b"));
        assert_eq!(
            json["edges"],
            serde_json::json!([{"from": "mod_a", "to": "mod_b"}])
        );
    }

    #[tokio::test]
    async fn get_graph_modules_has_no_edge_for_an_import_within_the_same_module() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_import_edge(&root);

        let (status, json) = get(root, "/api/graph-modules").await;

        assert_eq!(status, StatusCode::OK);
        let nodes: Vec<&str> = json["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert_eq!(nodes, vec!["."], "both root-level files share module \".\"");
        assert_eq!(json["edges"], serde_json::json!([]));
    }

    /// Two triangles of mutually-importing files joined by a single
    /// bridge import -- the same canonical toy case
    /// `repowise_graph::community`'s own tests use, built here as a
    /// real `RepoIndex` to exercise `/api/communities`' server-side
    /// wiring (line-count sizing, dominant-language labeling, relative
    /// paths) rather than the algorithm itself.
    fn index_with_two_triangle_clusters(root: &Path) -> RepoIndex {
        let names = ["a1", "a2", "a3", "b1", "b2", "b3"];
        for name in names {
            std::fs::write(root.join(format!("{name}.rs")), "// f\n").unwrap();
        }
        let import = |to: &str| repowise_core::ImportRef {
            path: to.to_string(),
            line: 1,
            resolved_file: Some(root.join(format!("{to}.rs"))),
        };
        let file = |name: &str, imports: Vec<repowise_core::ImportRef>, lines: usize| {
            repowise_core::FileRecord {
                path: root.join(format!("{name}.rs")),
                language: repowise_core::Language::Rust,
                lines,
                symbols: vec![],
                imports,
                calls: vec![],
                field_accesses: vec![],
            }
        };
        let index = RepoIndex {
            root: root.to_path_buf(),
            files: vec![
                file("a1", vec![import("a2"), import("a3")], 10),
                file("a2", vec![], 20),
                file("a3", vec![], 30),
                file("b1", vec![import("b2"), import("b3")], 40),
                file("b2", vec![], 50),
                file("b3", vec![import("a1")], 60),
            ],
            other_files: 0,
            indexed_commit: None,
        };
        index.save(root).unwrap();
        index
    }

    #[tokio::test]
    async fn get_communities_splits_two_bridged_triangles_and_sizes_by_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_two_triangle_clusters(&root);

        let (status, json) = get(root, "/api/communities").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["truncated"], false);
        let communities = json["communities"].as_array().unwrap();
        assert_eq!(communities.len(), 2, "{communities:?}");
        for community in communities {
            assert_eq!(community["file_count"], 3);
            assert_eq!(community["dominant_language"], "Rust");
            let files = community["files"].as_array().unwrap();
            assert_eq!(files.len(), 3);
        }
        let total_lines: i64 = communities
            .iter()
            .map(|c| c["total_lines"].as_i64().unwrap())
            .sum();
        assert_eq!(total_lines, 10 + 20 + 30 + 40 + 50 + 60);
    }

    #[tokio::test]
    async fn get_communities_with_no_imports_is_one_community_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/communities").await;

        assert_eq!(status, StatusCode::OK);
        let communities = json["communities"].as_array().unwrap();
        assert_eq!(communities.len(), 1, "{communities:?}");
        assert_eq!(communities[0]["file_count"], 1);
        assert_eq!(communities[0]["files"], serde_json::json!(["busy.rs"]));
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
    async fn get_coupling_reports_unavailable_without_git_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/coupling").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["pairs"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_coupling_ranks_the_most_co_changed_pair_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        git_commit_all(&root, "add a and b together");
        std::fs::write(root.join("a.txt"), "a2\n").unwrap();
        std::fs::write(root.join("b.txt"), "b2\n").unwrap();
        git_commit_all(&root, "change a and b together again");
        std::fs::write(root.join("c.txt"), "c\n").unwrap();
        std::fs::write(root.join("a.txt"), "a3\n").unwrap();
        git_commit_all(&root, "change a and c together once");
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/coupling").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], true);
        let pairs = json["pairs"].as_array().unwrap();
        assert!(!pairs.is_empty());
        assert_eq!(pairs[0]["file_a"], "a.txt");
        assert_eq!(pairs[0]["file_b"], "b.txt");
        assert_eq!(pairs[0]["count"], 2);
    }

    #[tokio::test]
    async fn get_commits_lists_newest_first_with_no_risk_score() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        git_commit_all(&root, "first");
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        git_commit_all(&root, "second");

        let (status, json) = get(root, "/api/commits").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], true);
        let commits = json["commits"].as_array().unwrap();
        assert_eq!(commits.len(), 2, "{commits:?}");
        assert_eq!(commits[0]["message"], "second");
        assert_eq!(commits[1]["message"], "first");
        assert_eq!(commits[0]["files_touched"], 1);
        assert!(commits[0]["short_hash"].as_str().unwrap().len() == 7);
        assert!(commits[0].get("score").is_none(), "{commits:?}");
    }

    #[tokio::test]
    async fn get_commits_respects_the_limit_query_param() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        git_commit_all(&root, "first");
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        git_commit_all(&root, "second");

        let (status, json) = get(root, "/api/commits?limit=1").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["commits"].as_array().unwrap().len(), 1);
        assert_eq!(json["commits"][0]["message"], "second");
    }

    #[tokio::test]
    async fn get_commits_is_unavailable_without_git_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/commits").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["commits"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_commit_risk_scores_the_head_commit_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        git_commit_all(&root, "add a");

        let (status, json) = get(root, "/api/commit-risk").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["revspec"], "HEAD");
        assert_eq!(json["files_touched"], 1);
        assert!(json["score"].as_f64().unwrap() >= 0.0);
    }

    #[tokio::test]
    async fn get_commit_risk_errors_when_not_a_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let (status, _json) = get(root, "/api/commit-risk").await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn get_external_deps_reports_a_cargo_dependency() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1.0\"\n",
        )
        .unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/external-deps").await;

        assert_eq!(status, StatusCode::OK);
        let deps = json.as_array().unwrap();
        assert_eq!(deps.len(), 1, "{deps:?}");
        assert_eq!(deps[0]["name"], "serde");
        assert_eq!(deps[0]["version"], "1.0");
        assert_eq!(deps[0]["kind"], "direct");
        assert_eq!(deps[0]["ecosystem"], "cargo");
        assert_eq!(deps[0]["file"], "Cargo.toml");
    }

    #[tokio::test]
    async fn get_external_deps_is_an_empty_list_with_no_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/external-deps").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!([]));
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
    async fn get_settings_reports_default_health_weights_with_no_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/settings").await;

        assert_eq!(status, StatusCode::OK);
        let toml_text = json["health_weights_toml"].as_str().unwrap();
        assert!(toml_text.contains("[health_weights]"));
        let parsed: RepoConfig = toml::from_str(toml_text).unwrap();
        assert_eq!(
            parsed.health_weights.long_function,
            repowise_health::HealthWeights::default().long_function
        );
    }

    #[tokio::test]
    async fn post_settings_health_weights_persists_and_is_reflected_back() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        let router = app(root.clone(), None, None);

        let body = serde_json::json!({
            "toml": "[health_weights]\nlong_function = 1.5\n"
        });
        let (status, json) = post_json(router, "/api/settings/health-weights", body).await;

        assert_eq!(status, StatusCode::OK);
        let toml_text = json["health_weights_toml"].as_str().unwrap();
        let parsed: RepoConfig = toml::from_str(toml_text).unwrap();
        assert_eq!(parsed.health_weights.long_function, 1.5);
        assert!(repo_config_path(&root).exists());
    }

    #[tokio::test]
    async fn post_settings_health_weights_rejects_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        let router = app(root.clone(), None, None);

        let body = serde_json::json!({ "toml": "not valid toml {{{" });
        let (status, _json) = post_json(router, "/api/settings/health-weights", body).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !repo_config_path(&root).exists(),
            "an invalid submission must not be written to disk"
        );
    }

    #[tokio::test]
    async fn a_persisted_health_weight_override_changes_the_reported_health_score() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (_status, baseline) = get(root.clone(), "/api/health").await;
        let baseline_score = baseline["worst_files"][0]["score"].as_f64().unwrap();

        std::fs::create_dir_all(root.join(".repowise")).unwrap();
        std::fs::write(
            repo_config_path(&root),
            "[health_weights]\nhigh_complexity = 100.0\n",
        )
        .unwrap();

        let (status, overridden) = get(root, "/api/health").await;

        assert_eq!(status, StatusCode::OK);
        let overridden_score = overridden["worst_files"][0]["score"].as_f64().unwrap();
        assert!(
            overridden_score < baseline_score,
            "a much larger high-complexity penalty must lower the score \
             (baseline {baseline_score}, overridden {overridden_score})"
        );
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

    /// Two files that import each other -- the same shape
    /// `repowise-refactor`'s own `an_import_cycle_becomes_a_break_import_cycle_candidate`
    /// test uses, re-expressed as pre-resolved `ImportRef`s so the
    /// fixture doesn't depend on any language-specific import parsing.
    fn index_with_an_import_cycle(root: &Path) -> RepoIndex {
        let a = root.join("a.rs");
        let b = root.join("b.rs");
        std::fs::write(&a, "mod b;\n").unwrap();
        std::fs::write(&b, "mod a;\n").unwrap();
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
                    imports: vec![repowise_core::ImportRef {
                        path: "a".to_string(),
                        line: 1,
                        resolved_file: Some(a),
                    }],
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

    #[tokio::test]
    async fn get_refactor_candidates_returns_an_import_cycle_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_an_import_cycle(&root);

        let (status, json) = get(root, "/api/refactor-candidates").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["total_matching"], 1);
        assert_eq!(json["candidates"][0]["kind"], "break-import-cycle");
        assert_eq!(json["candidates"][0]["files"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn get_refactor_candidates_filters_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_an_import_cycle(&root);

        let (status, json) = get(root, "/api/refactor-candidates?kind=split-god-class").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["total_matching"], 0);
        assert_eq!(json["candidates"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_refactor_candidates_errors_on_invalid_kind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_an_import_cycle(&root);

        let (status, _json) = get(root, "/api/refactor-candidates?kind=nonsense").await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn get_security_finds_a_hardcoded_aws_key_and_never_echoes_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let index = index_with_one_busy_symbol(&root);
        std::fs::write(
            &index.files[0].path,
            "pub fn busy() {}\nlet key = \"AKIAABCDEFGHIJKLMNOP\";\n",
        )
        .unwrap();

        let (status, json) = get(root, "/api/security").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["total_matching"], 1);
        let findings = json["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["kind"], "aws-access-key-id");
        assert_eq!(findings[0]["severity"], "high");
        assert_eq!(findings[0]["line"], 2);
        assert_eq!(findings[0]["file"], "busy.rs");
        let message = findings[0]["message"].as_str().unwrap();
        assert!(
            !message.contains("AKIA"),
            "must never echo the secret: {message}"
        );
    }

    #[tokio::test]
    async fn get_security_reports_no_findings_for_clean_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/security").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["total_matching"], 0);
        assert_eq!(json["findings"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_security_filters_by_min_severity() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let index = index_with_one_busy_symbol(&root);
        std::fs::write(
            &index.files[0].path,
            "let key = \"AKIAABCDEFGHIJKLMNOP\";\napi_key = \"sk_live_9f8a7b6c5d4e3f2a1b0c\"\n",
        )
        .unwrap();

        let (status, json) = get(root.clone(), "/api/security?min_severity=high").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["total_matching"], 1);
        assert_eq!(json["findings"][0]["kind"], "aws-access-key-id");

        let (status, json) = get(root, "/api/security?min_severity=medium").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["total_matching"], 2);
    }

    #[tokio::test]
    async fn get_security_errors_on_an_invalid_min_severity() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, _json) = get(root, "/api/security?min_severity=critical").await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn get_doc_coverage_reports_missing_for_a_file_with_no_wiki_page() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/doc-coverage").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["missing"], 1);
        assert_eq!(json["fresh"], 0);
        assert_eq!(json["stale"], 0);
        assert_eq!(json["entries"][0]["status"], "missing");
    }

    #[tokio::test]
    async fn get_doc_coverage_reports_fresh_right_after_generation_and_stale_after_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let index = index_with_one_busy_symbol(&root);
        let graph = repowise_graph::RepoGraph::build(&index);
        let health = repowise_health::analyze(&index, &graph);
        repowise_docs::generate(&index, &graph, &health).unwrap();

        let (status, json) = get(root.clone(), "/api/doc-coverage").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["fresh"], 1);
        assert_eq!(json["entries"][0]["status"], "fresh");

        // Edit the source without regenerating -- the page now describes
        // stale content.
        std::fs::write(&index.files[0].path, "pub fn busy() { /* changed */ }\n").unwrap();

        let (status, json) = get(root, "/api/doc-coverage").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["stale"], 1);
        assert_eq!(json["entries"][0]["status"], "stale");
    }

    #[tokio::test]
    async fn get_saved_reports_zero_totals_with_no_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/saved").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["by"], "program");
        assert_eq!(json["distilled_runs"], 0);
        assert_eq!(json["saved_bytes"], 0);
        assert_eq!(json["groups"].as_array().unwrap().len(), 0);
        assert_eq!(json["mcp_tools"].as_array().unwrap().len(), 0);
        assert_eq!(json["missed"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_saved_groups_measured_savings_by_program_and_keeps_mcp_modelled_separate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        let store_dir = repowise_distill::store::store_dir(&root, None);

        repowise_distill::ledger::record_distilled(&store_dir, "cargo", 4000, 400, 0);
        repowise_distill::ledger::record_distilled(&store_dir, "cargo", 2000, 200, 0);
        repowise_distill::ledger::record_distilled(&store_dir, "pytest", 1000, 100, 0);
        repowise_distill::ledger::record_mcp_response(&store_dir, "get_context", 9000, 900);
        repowise_distill::ledger::record_skipped(&store_dir, "git", "not-rewritable");

        let (status, json) = get(root, "/api/saved").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["distilled_runs"], 3);
        // Measured total must not include the modelled MCP saving (8100).
        assert_eq!(json["raw_bytes"], 7000);
        assert_eq!(json["kept_bytes"], 700);
        assert_eq!(json["saved_bytes"], 6300);

        let groups = json["groups"].as_array().unwrap();
        let cargo_group = groups.iter().find(|g| g["key"] == "cargo").unwrap();
        assert_eq!(cargo_group["runs"], 2);
        assert_eq!(cargo_group["saved_bytes"], 5400);
        let pytest_group = groups.iter().find(|g| g["key"] == "pytest").unwrap();
        assert_eq!(pytest_group["runs"], 1);
        assert_eq!(pytest_group["saved_bytes"], 900);

        assert_eq!(json["mcp_baseline_bytes"], 9000);
        assert_eq!(json["mcp_avoided_bytes"], 8100);
        let mcp_tools = json["mcp_tools"].as_array().unwrap();
        assert_eq!(mcp_tools.len(), 1);
        assert_eq!(mcp_tools[0]["tool"], "get_context");
        assert_eq!(mcp_tools[0]["calls"], 1);

        let missed = json["missed"].as_array().unwrap();
        assert_eq!(missed.len(), 1);
        assert_eq!(missed[0]["program"], "git");
        assert_eq!(missed[0]["reason"], "not-rewritable");
        assert_eq!(missed[0]["count"], 1);
    }

    #[tokio::test]
    async fn get_saved_can_group_by_day() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        let store_dir = repowise_distill::store::store_dir(&root, None);
        repowise_distill::ledger::record_distilled(&store_dir, "cargo", 100, 10, 0);

        let (status, json) = get(root, "/api/saved?by=day").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["by"], "day");
        let groups = json["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert!(groups[0]["key"].as_str().unwrap().starts_with("day "));
    }

    #[tokio::test]
    async fn get_saved_errors_on_an_invalid_by_value() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, _json) = get(root, "/api/saved?by=hour").await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn get_saved_flags_mcp_calls_that_returned_more_than_they_covered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);
        let store_dir = repowise_distill::store::store_dir(&root, None);
        // The curated answer (500 bytes) is bigger than the baseline it
        // covered (200 bytes) -- a real cost, not a saving.
        repowise_distill::ledger::record_mcp_response(&store_dir, "get_symbol", 200, 500);

        let (status, json) = get(root, "/api/saved").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["mcp_costlier_calls"], 1);
        assert_eq!(json["mcp_overhead_bytes"], 300);
        assert_eq!(json["mcp_avoided_bytes"], 0);
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
            workspace_state_dir: Arc::new(None),
            reindex_job: ReindexJob::new(),
            usage: UsageTracker::new(),
            webhook_secret: Arc::new(None),
        };
        build_router(state, None)
    }

    fn app_with_webhook_secret(root: PathBuf, secret: Option<&str>) -> Router {
        let state = AppState {
            root: Arc::new(root),
            llm_config: Arc::new(None),
            workspace_repos: Arc::new(None),
            workspace_state_dir: Arc::new(None),
            reindex_job: ReindexJob::new(),
            usage: UsageTracker::new(),
            webhook_secret: Arc::new(secret.map(str::to_string)),
        };
        build_router(state, None)
    }

    async fn get_with_router(router: Router, uri: &str) -> (StatusCode, serde_json::Value) {
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

    /// A raw `POST` with one custom header -- webhook receivers care
    /// about a specific header's presence/value, not a JSON content
    /// type, so this deliberately doesn't reuse `post_json`.
    async fn post_with_header(
        router: Router,
        uri: &str,
        header_name: &str,
        header_value: &str,
        body: &[u8],
    ) -> (StatusCode, String) {
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header(header_name, header_value)
                    .body(axum::body::Body::from(body.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
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
    async fn get_search_semantic_reports_unavailable_without_llm_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let (status, json) = get(root, "/api/search-semantic?q=busy").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert_eq!(json["files"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_search_semantic_reports_unavailable_for_an_empty_query_without_calling_the_llm() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let server = ChatFixtureServer::start_sequence(vec![]);
        let config = repowise_llm::LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };
        let router = app_with_llm_config(root, Some(config));
        let (status, json) = get_with_router(router, "/api/search-semantic?q=").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], false);
        assert!(
            server.requests().is_empty(),
            "an empty query must not call the LLM endpoint at all"
        );
    }

    #[tokio::test]
    async fn get_search_semantic_returns_cited_files_when_llm_is_configured() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let embeddings_response =
            r#"{"data": [{"embedding": [1.0, 0.0]}, {"embedding": [1.0, 0.0]}]}"#;
        let server = ChatFixtureServer::start_sequence(vec![embeddings_response]);
        let config = repowise_llm::LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let router = app_with_llm_config(root, Some(config));
        let (status, json) =
            get_with_router(router, "/api/search-semantic?q=what+handles+auth").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["available"], true);
        assert_eq!(json["files"], serde_json::json!(["busy.rs"]));
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
        assert_eq!(json["broken"], serde_json::json!([]));
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
        assert_eq!(json["broken"], serde_json::json!([]));
    }

    /// Two calls against the same `--workspace` flag (so the same
    /// `.repowise-workspace/contracts.json` snapshot on disk), with the
    /// producer's route removed in between -- exercises
    /// `workspace_contract_changes` through the real HTTP surface, not
    /// just `repowise-workspace`'s own unit tests.
    #[tokio::test]
    async fn get_workspace_contracts_reports_a_broken_contract_after_the_producer_route_disappears()
    {
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

        // First call: matches, and (since `app()` is built fresh here,
        // with no prior snapshot on disk) nothing broken yet.
        let router = app(root.clone(), None, Some(workspace_path.clone()));
        let (status, json) = {
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/api/workspace-contracts")
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
        assert_eq!(json["matches"].as_array().unwrap().len(), 1);
        assert_eq!(json["broken"], serde_json::json!([]));

        // The producer drops its route entirely.
        std::fs::write(server_repo.join("routes.rs"), "// no routes here\n").unwrap();
        let index = RepoIndex {
            root: server_repo.clone(),
            files: vec![],
            other_files: 1,
            indexed_commit: None,
        };
        index.save(&server_repo).unwrap();

        // A fresh `app()` (new process, same workspace file) still reads
        // the snapshot the first call wrote -- the baseline lives on
        // disk, not in server memory.
        let router = app(root, None, Some(workspace_path));
        let (status, json) = {
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/api/workspace-contracts")
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
        assert_eq!(json["matches"], serde_json::json!([]));
        let broken = json["broken"].as_array().unwrap();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0]["path"], "/api/hotspots");
        assert_eq!(broken[0]["consumer_repo"], "client");
        assert_eq!(broken[0]["previous_producer_repo"], "server");
        assert_eq!(broken[0]["reason"], "no-producer-anywhere");
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

    fn github_signature(secret: &str, body: &[u8]) -> String {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let tag = ring::hmac::sign(&key, body);
        let hex: String = tag.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256={hex}")
    }

    #[tokio::test]
    async fn post_webhook_github_is_unavailable_without_a_configured_secret() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let router = app_with_webhook_secret(root, None);
        let (status, _body) = post_with_header(
            router,
            "/api/webhook/github",
            "x-hub-signature-256",
            "sha256=deadbeef",
            b"{}",
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn post_webhook_github_rejects_an_invalid_signature() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let router = app_with_webhook_secret(root, Some("shh"));
        let (status, _body) = post_with_header(
            router,
            "/api/webhook/github",
            "x-hub-signature-256",
            "sha256=0000000000000000000000000000000000000000000000000000000000000000",
            b"{}",
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn post_webhook_github_triggers_a_reindex_on_a_valid_signature() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let secret = "shh";
        let body = br#"{"ref": "refs/heads/main"}"#;
        let signature = github_signature(secret, body);

        let router = app_with_webhook_secret(root, Some(secret));
        let (status, response_body) = post_with_header(
            router.clone(),
            "/api/webhook/github",
            "x-hub-signature-256",
            &signature,
            body,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&response_body).unwrap();
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
    }

    #[tokio::test]
    async fn post_webhook_gitlab_is_unavailable_without_a_configured_secret() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let router = app_with_webhook_secret(root, None);
        let (status, _body) = post_with_header(
            router,
            "/api/webhook/gitlab",
            "x-gitlab-token",
            "whatever",
            b"",
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn post_webhook_gitlab_rejects_an_incorrect_token() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let router = app_with_webhook_secret(root, Some("shh"));
        let (status, _body) = post_with_header(
            router,
            "/api/webhook/gitlab",
            "x-gitlab-token",
            "not-the-secret",
            b"",
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn post_webhook_gitlab_triggers_a_reindex_on_the_correct_token() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        index_with_one_busy_symbol(&root);

        let router = app_with_webhook_secret(root, Some("shh"));
        let (status, response_body) = post_with_header(
            router.clone(),
            "/api/webhook/gitlab",
            "x-gitlab-token",
            "shh",
            b"",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&response_body).unwrap();
        assert!(json["status"] == "running" || json["status"] == "completed");
    }

    #[test]
    fn verify_gitlab_token_rejects_different_length_secrets() {
        assert!(!verify_gitlab_token("shh", "shhh"));
        assert!(!verify_gitlab_token("shh", "sh"));
    }

    #[test]
    fn verify_gitlab_token_accepts_only_an_exact_match() {
        assert!(verify_gitlab_token("shh", "shh"));
        assert!(!verify_gitlab_token("shh", "SHH"));
    }

    #[test]
    fn decode_hex_rejects_odd_length_and_non_hex_input() {
        assert_eq!(decode_hex("deadbeef"), Some(vec![0xde, 0xad, 0xbe, 0xef]));
        assert_eq!(decode_hex("abc"), None);
        assert_eq!(decode_hex("zz"), None);
    }
}
