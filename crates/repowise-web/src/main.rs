//! `WorkspaceSection` is issue #64's first slice (multi-repo/workspace
//! support): a read-only "repo cards" view over `GET
//! /api/workspace-repos`, listing every repo the server was started
//! with (`--workspace <path>`) and each one's indexed status. Shows an
//! explanatory message instead of empty cards when no workspace was
//! configured. Deliberately doesn't let you switch which repo the rest
//! of the dashboard is viewing.
//!
//! `CoChangesSection` is the next #64 slice: each workspace repo's own
//! most-coupled file pairs over `GET /api/workspace-co-changes`, shown
//! side by side. Not cross-repo co-change -- separate repos have
//! separate git histories, so files in different repos can never
//! literally co-change together -- just each repo's own coupling
//! rendered together in one view.
//!
//! `SystemMapSection` is the next #64 slice: real cross-repo import
//! resolution over `GET /api/workspace-architecture`, rendered as a
//! plain repo-pair table with the individual import sites listed
//! underneath. Covers Rust, Python, Java, Kotlin, Scala, Go, C#, and PHP.
//!
//! `ConformanceSection` is the next #64 slice: circular cross-repo
//! dependencies over `GET /api/workspace-conformance`, reusing exactly
//! the edges `SystemMapSection` already renders -- a workspace's
//! repo-level dependency graph should form a DAG, so a cycle is a
//! concrete "pattern divergence" finding needing no further
//! human-specified rule set.
//!
//! `ContractsSection` is the last of #64's five bundled items: regex-
//! based HTTP producer/consumer route matching over `GET
//! /api/workspace-contracts`. Fully independent of the other #64 views
//! -- no cross-repo symbol resolution involved, just a fixed pattern
//! table over raw source text (see `repowise-workspace`'s `contracts`
//! module doc comment for the coarse/heuristic caveat).
//!
//! Issue #65 (live-server-dependent dashboard features) also tracks
//! cost tracking, its fifth and last bundled feature: `UsageSection`
//! polls `GET /api/usage` every 3s for running chat-call and
//! prompt/completion/total token counts, tallied for this server
//! process only (not a persisted history across restarts) and in
//! tokens, not a dollar figure -- see `repowise-llm`'s own doc comment
//! for why there's no pricing conversion. Polling (rather than a
//! one-shot fetch) is what lets it reflect `ChatSection`'s activity
//! without the two components sharing state directly.
//!
//! It also tracks a read-only Settings view (`SettingsSection`, over
//! `GET /api/settings`): repo root, indexed file counts, and whether
//! git history/wiki pages/an LLM are available. No edit form -- this
//! port has no persisted config to write to yet, so it's a status
//! view, not a settings *editor*.
//!
//! It also tracks a live job banner: a "Reindex" button (`JobBanner`)
//! that triggers the server's `POST /api/reindex` and polls `GET
//! /api/reindex` (via `gloo-timers`) until the background job leaves
//! `Running`, picking up an already-in-flight job on page load too.
//!
//! It also tracks Present Mode: a full-screen, keyboard-driven step-
//! through of Overview/Health/Hotspots/Decisions/Graph, with the current
//! slide reflected in the URL hash (`#present/<n>`) so a link to a
//! specific slide is shareable/bookmarkable, matching the issue's own
//! framing. Purely a frontend feature -- no new server endpoints,
//! reusing the same section components and data every other view
//! already fetches.
//!
//! Phase 5 of the #59/#65 dashboard-server pivot added the last
//! *static-parity* view: a chat section over `/api/chat`, an opt-in
//! endpoint now grounded by real embeddings-based retrieval (issue #63
//! -- see `repowise-server`'s own module doc for the retrieval/
//! fallback design). Shows a plain explanatory message instead of a
//! chat box when the server reports the feature isn't configured.
//! Phase 4 broadened every file-path
//! drill-down (Phase 2's wiki-only links) into a file-detail panel:
//! wiki page, git-blame ownership breakdown, and any linked
//! architectural decisions, each independently "not available" rather
//! than a shared error -- so every indexed file is clickable now, not
//! just ones with a wiki page. It also added a dead-code section
//! (`/api/dead-code`, with a minimum-confidence filter). Phase 3 added
//! a dependency-graph view: an SVG rendering of `/api/graph`'s
//! file-level import graph, laid out client-side with a small
//! force-directed simulation (no D3 or other JS graph library --
//! keeping the whole frontend buildable with just `cargo`/`trunk`).

use gloo_timers::future::TimeoutFuture;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos::wasm_bindgen::JsCast;
use leptos::web_sys;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone, Debug)]
struct Overview {
    file_count: usize,
    other_file_count: usize,
    total_lines: usize,
    import_edges: usize,
    call_edges: usize,
    most_depended_on: Vec<(String, usize)>,
}

#[derive(Deserialize, Clone, Debug)]
struct FindingKindCount {
    kind: String,
    count: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct FileHealth {
    file: String,
    score: f64,
    finding_count: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct Health {
    average_score: f64,
    file_count: usize,
    finding_count: usize,
    by_kind: Vec<FindingKindCount>,
    worst_files: Vec<FileHealth>,
}

#[derive(Deserialize, Clone, Debug)]
struct SymbolDetail {
    found: bool,
    name: String,
    kind: String,
    file: String,
    start_line: usize,
    end_line: usize,
    parent: Option<String>,
    complexity: usize,
    max_nesting_depth: usize,
    callees: Vec<String>,
    callers: Vec<String>,
    unresolved_callee_count: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct DecisionDetail {
    found: bool,
    id: String,
    title: String,
    status: Option<String>,
    superseded_by: Option<String>,
    supersedes: Option<String>,
    body: String,
    linked_files: Vec<String>,
    source: String,
    inferred: bool,
}

#[derive(Deserialize, Clone, Debug)]
struct Stats {
    available: bool,
    shallow: bool,
    commit_count: usize,
    punch_card: Vec<Vec<usize>>,
    weekly_trend: Vec<usize>,
    timezone: String,
}

const DAY_LABELS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

#[derive(Deserialize, Clone, Debug)]
struct FileEntry {
    path: String,
    language: String,
    lines: usize,
    score: Option<f64>,
    finding_count: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct Files {
    files: Vec<FileEntry>,
    total_lines: usize,
    health_available: bool,
}

/// One laid-out treemap tile, in SVG user units.
#[derive(Clone, Debug, PartialEq)]
struct Tile {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    index: usize,
}

/// Squarified treemap layout (Bruls, Huizing & van Wijk).
///
/// Chosen over slice-and-dice because slice-and-dice degenerates into
/// slivers as the item count grows -- unreadable at 85 files, let alone
/// more. Squarified keeps tiles near-square, which is what makes the
/// area comparison legible in the first place.
///
/// Written by hand rather than pulled from a crate: this is the whole
/// algorithm, and a WASM binary should not grow a charting dependency
/// for ~60 lines.
///
/// Pure and deterministic -- same `values` in the same order gives the
/// same tiles, which is what stops the view reshuffling between loads.
/// `values` must be sorted descending by the caller.
fn squarify(values: &[f64], width: f64, height: f64) -> Vec<Tile> {
    let total: f64 = values.iter().sum();
    let mut tiles = Vec::new();
    if total <= 0.0 || width <= 0.0 || height <= 0.0 {
        return tiles;
    }

    // Work in area units scaled so the values fill the rectangle.
    let scale = width * height / total;
    let mut rect = (0.0f64, 0.0f64, width, height); // x, y, w, h
    let mut i = 0usize;

    while i < values.len() {
        let (rx, ry, rw, rh) = rect;
        if rw <= 0.0 || rh <= 0.0 {
            break;
        }
        let short = rw.min(rh);

        // Grow a row while doing so improves the worst aspect ratio.
        let mut row_end = i;
        let mut row_sum = 0.0f64;
        let mut best = f64::INFINITY;
        while row_end < values.len() {
            let candidate_sum = row_sum + values[row_end] * scale;
            let row = &values[i..=row_end];
            let worst = worst_ratio(row, scale, candidate_sum, short);
            if worst > best {
                break;
            }
            best = worst;
            row_sum = candidate_sum;
            row_end += 1;
        }

        // Lay the row along the shorter side.
        let thickness = if short > 0.0 { row_sum / short } else { 0.0 };
        let mut offset = 0.0f64;
        for (n, v) in values[i..row_end].iter().enumerate() {
            let area = v * scale;
            let length = if row_sum > 0.0 { area / thickness } else { 0.0 };
            let tile = if rw >= rh {
                Tile {
                    x: rx,
                    y: ry + offset,
                    w: thickness,
                    h: length,
                    index: i + n,
                }
            } else {
                Tile {
                    x: rx + offset,
                    y: ry,
                    w: length,
                    h: thickness,
                    index: i + n,
                }
            };
            offset += length;
            tiles.push(tile);
        }

        rect = if rw >= rh {
            (rx + thickness, ry, rw - thickness, rh)
        } else {
            (rx, ry + thickness, rw, rh - thickness)
        };
        i = row_end;
    }
    tiles
}

/// Worst (largest) aspect ratio in a candidate row.
fn worst_ratio(row: &[f64], scale: f64, row_sum: f64, short: f64) -> f64 {
    if row_sum <= 0.0 || short <= 0.0 {
        return f64::INFINITY;
    }
    let areas: Vec<f64> = row.iter().map(|v| v * scale).collect();
    let max = areas.iter().cloned().fold(f64::MIN, f64::max);
    let min = areas.iter().cloned().fold(f64::MAX, f64::min);
    let s2 = row_sum * row_sum;
    let short2 = short * short;
    ((short2 * max) / s2).max(s2 / (short2 * min))
}

/// Health band for a file score, as (label, fill).
///
/// The label matters as much as the fill: color alone is not an
/// accessible channel, so every tile carries its band in text via the
/// SVG `<title>`, and the legend names the bands rather than only
/// showing swatches.
fn health_band(score: Option<f64>) -> (&'static str, &'static str) {
    match score {
        // Unscored is its own band, never folded into "good" -- a file
        // with no score is not a healthy file.
        None => ("unscored", "#9e9e9e"),
        Some(s) if s >= 8.0 => ("good", "#2e7d32"),
        Some(s) if s >= 5.0 => ("fair", "#f9a825"),
        _ => ("poor", "#c62828"),
    }
}

#[derive(Deserialize, Clone, Debug)]
struct Contributor {
    author: String,
    lines_owned: usize,
    percent: f64,
    files_touched: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct Contributors {
    available: bool,
    contributors: Vec<Contributor>,
    bus_factor_distribution: Vec<(usize, usize)>,
    files_sampled: usize,
    files_total: usize,
    limit_applied: bool,
    files_unblameable: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct FileCoverage {
    path: String,
    percent: f64,
    lines_known: usize,
    lines_hit: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct Coverage {
    available: bool,
    files: Vec<FileCoverage>,
    unmeasured_files: usize,
    mean_percent: f64,
    has_per_test_map: bool,
    test_contexts: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct Hotspot {
    file: String,
    churn: usize,
    total_complexity: usize,
    bugfix_commits: usize,
    score: usize,
    decayed_score: f64,
}

#[derive(Deserialize, Clone, Debug)]
struct Hotspots {
    available: bool,
    hotspots: Vec<Hotspot>,
}

#[derive(Deserialize, Clone, Debug)]
struct Decision {
    id: String,
    title: String,
    status: Option<String>,
    superseded_by: Option<String>,
    linked_file_count: usize,
    source: String,
    /// True when a model inferred this from code rather than reading it
    /// from something a person wrote. Rendered as a badge -- carrying it
    /// in the payload without showing it would defeat the point.
    inferred: bool,
}

/// `GET /api/decisions`. An object rather than a bare array so it can
/// carry `inferred_source`: an empty list is ambiguous between "no
/// inferred decisions here" and "the pass that infers them never ran".
#[derive(Deserialize, Clone, Debug)]
struct Decisions {
    decisions: Vec<Decision>,
    inferred_source: String,
}

#[derive(Deserialize, Clone, Debug)]
struct Symbol {
    name: String,
    kind: String,
    file: String,
    start_line: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct WikiPage {
    #[allow(dead_code)]
    path: String,
    content: String,
}

#[derive(Deserialize, Clone, Debug)]
struct SearchResults {
    files: Vec<String>,
    symbols: Vec<Symbol>,
}

#[derive(Deserialize, Clone, Debug)]
struct GraphNode {
    id: String,
    language: String,
}

#[derive(Deserialize, Clone, Debug)]
struct GraphEdge {
    from: String,
    to: String,
}

#[derive(Deserialize, Clone, Debug)]
struct Graph {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    truncated: bool,
}

#[derive(Deserialize, Clone, Debug)]
struct OwnershipEntry {
    author: String,
    lines: usize,
    percentage: f64,
}

#[derive(Deserialize, Clone, Debug)]
struct Ownership {
    available: bool,
    owners: Vec<OwnershipEntry>,
}

#[derive(Deserialize, Clone, Debug)]
struct DeadCodeCandidate {
    file: String,
    symbol: String,
    line: usize,
    confidence: String,
    risk_factors: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
struct DeadCode {
    candidates: Vec<DeadCodeCandidate>,
    total_matching: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct RefactorCandidate {
    id: String,
    kind: String,
    title: String,
    rationale: String,
    files: Vec<String>,
    symbols: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
struct RefactorCandidates {
    candidates: Vec<RefactorCandidate>,
    total_matching: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct DocCoverageEntry {
    file: String,
    /// `"missing"`, `"fresh"`, or `"stale"`.
    status: String,
}

#[derive(Deserialize, Clone, Debug)]
struct DocCoverage {
    entries: Vec<DocCoverageEntry>,
    missing: usize,
    fresh: usize,
    stale: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct CouplingPair {
    file_a: String,
    file_b: String,
    count: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct Coupling {
    available: bool,
    pairs: Vec<CouplingPair>,
}

#[derive(Deserialize, Clone, Debug)]
struct ExternalDependency {
    name: String,
    version: Option<String>,
    /// `"direct"`, `"dev"`, or `"build"`.
    kind: String,
    /// `"cargo"`, `"npm"`, `"pypi"`, `"go"`, or `"composer"`.
    ecosystem: String,
    file: String,
}

#[derive(Deserialize, Clone, Debug)]
struct Commit {
    hash: String,
    short_hash: String,
    author: String,
    message: String,
    /// Unix seconds (author date).
    timestamp: i64,
    files_touched: usize,
}

#[derive(Deserialize, Clone, Debug)]
struct Commits {
    available: bool,
    commits: Vec<Commit>,
}

#[derive(Deserialize, Clone, Debug)]
struct CommitRisk {
    lines_added: usize,
    lines_deleted: usize,
    files_touched: usize,
    subsystems_touched: usize,
    concentration: f64,
    author: String,
    author_prior_commits: usize,
    score: f64,
}

#[derive(Deserialize, Clone, Debug)]
struct Community {
    id: usize,
    files: Vec<String>,
    file_count: usize,
    total_lines: usize,
    dominant_language: String,
}

#[derive(Deserialize, Clone, Debug)]
struct Communities {
    communities: Vec<Community>,
    truncated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ChatTurn {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    history: Vec<ChatTurn>,
}

#[derive(Deserialize, Clone, Debug)]
struct ChatResponse {
    available: bool,
    reply: Option<String>,
}

/// Mirrors `repowise-server`'s `SettingsDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct Settings {
    root: String,
    file_count: usize,
    other_file_count: usize,
    git_available: bool,
    wiki_pages_available: bool,
    llm_configured: bool,
    llm_model: Option<String>,
}

/// Mirrors `repowise-server`'s `WorkspaceRepoDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct WorkspaceRepo {
    name: String,
    path: String,
    indexed: bool,
    file_count: Option<usize>,
    other_file_count: Option<usize>,
}

/// Mirrors `repowise-server`'s `WorkspaceReposDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct WorkspaceRepos {
    available: bool,
    repos: Vec<WorkspaceRepo>,
}

/// Mirrors `repowise-server`'s `CoChangePairDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct CoChangePair {
    file_a: String,
    file_b: String,
    count: usize,
}

/// Mirrors `repowise-server`'s `RepoCoChangesDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct RepoCoChanges {
    name: String,
    available: bool,
    pairs: Vec<CoChangePair>,
}

/// Mirrors `repowise-server`'s `WorkspaceCoChangesDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct WorkspaceCoChanges {
    available: bool,
    repos: Vec<RepoCoChanges>,
}

/// Mirrors `repowise-server`'s `RepoEdgeSummaryDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct RepoEdgeSummary {
    from_repo: String,
    to_repo: String,
    edge_count: usize,
}

/// Mirrors `repowise-server`'s `CrossRepoEdgeDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct CrossRepoEdge {
    from_repo: String,
    from_file: String,
    to_repo: String,
    to_file: String,
    import_path: String,
}

/// Mirrors `repowise-server`'s `WorkspaceArchitectureDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct WorkspaceArchitecture {
    available: bool,
    repo_edges: Vec<RepoEdgeSummary>,
    edges: Vec<CrossRepoEdge>,
    total_edges: usize,
}

/// Mirrors `repowise-server`'s `WorkspaceConformanceDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct WorkspaceConformance {
    available: bool,
    cycles: Vec<Vec<String>>,
}

/// Mirrors `repowise-server`'s `ContractMatchDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct ContractMatch {
    producer_repo: String,
    producer_file: String,
    consumer_repo: String,
    consumer_file: String,
    path: String,
}

/// Mirrors `repowise-server`'s `UnmatchedConsumerDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct UnmatchedConsumer {
    repo: String,
    file: String,
    path: String,
}

/// Mirrors `repowise-server`'s `BrokenContractDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct BrokenContract {
    path: String,
    consumer_repo: String,
    consumer_file: String,
    previous_producer_repo: String,
    reason: Option<String>,
}

/// Mirrors `repowise-server`'s `WorkspaceContractsDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct WorkspaceContracts {
    available: bool,
    matches: Vec<ContractMatch>,
    unmatched_consumers: Vec<UnmatchedConsumer>,
    broken: Vec<BrokenContract>,
}

/// Mirrors `repowise-server`'s `UsageTotalsDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct Usage {
    chat_call_count: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

/// Mirrors `repowise-server`'s `ReindexStatusDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ReindexStatus {
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

async fn fetch_json<T>(path: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    fetch_json_with_query(path, &[]).await
}

async fn fetch_json_with_query<T>(path: &str, params: &[(&str, &str)]) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let response = gloo_net::http::Request::get(path)
        .query(params.iter().map(|(k, v)| (*k, *v)))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.ok() {
        return Err(format!("server returned {}", response.status()));
    }
    response.json::<T>().await.map_err(|e| e.to_string())
}

async fn post_json<Req, Resp>(path: &str, body: &Req) -> Result<Resp, String>
where
    Req: Serialize + ?Sized,
    Resp: for<'de> Deserialize<'de>,
{
    let response = gloo_net::http::Request::post(path)
        .json(body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.ok() {
        return Err(format!("server returned {}", response.status()));
    }
    response.json::<Resp>().await.map_err(|e| e.to_string())
}

type WikiPages = LocalResource<Result<Vec<String>, String>>;

fn wiki_pages_snapshot(wiki_pages: WikiPages) -> Vec<String> {
    wiki_pages
        .get()
        .and_then(|r| r.take().ok())
        .unwrap_or_default()
}

/// A file-path table cell: a link that opens the file-detail panel
/// (wiki page, git-blame ownership, and any linked architectural
/// decisions -- each independently "not available" rather than an
/// error when there's nothing there) via `selected`. Every indexed file
/// is clickable, unlike Phase 2's wiki-only gating: ownership and
/// linked decisions can be useful even for a file with no wiki page.
fn file_cell(path: String, selected: RwSignal<Option<String>>) -> impl IntoView {
    let target = path.clone();
    view! {
        <a href="#" on:click=move |ev| {
            ev.prevent_default();
            selected.set(Some(target.clone()));
        }>{path}</a>
    }
}

/// Every section below follows the same shape: fetch its own resource,
/// show a loading placeholder via `Suspense`, then render either the
/// data or an error -- mirroring the static dashboard's one-section-at-
/// a-time layout, but each section now loads independently instead of
/// blocking on a single whole-page render.
#[component]
fn OverviewSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let overview = LocalResource::new(|| fetch_json::<Overview>("/api/overview"));

    view! {
        <h2>"Overview"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                overview
                    .get()
                    .map(|result| match result.take() {
                        Ok(o) => view! {
                            <ul>
                                <li>{format!("{} indexed file(s)", o.file_count)}</li>
                                <li>{format!("{} other file(s)", o.other_file_count)}</li>
                                <li>{format!("{} total lines", o.total_lines)}</li>
                                <li>{format!("{} import edge(s)", o.import_edges)}</li>
                                <li>{format!("{} call edge(s)", o.call_edges)}</li>
                            </ul>
                            {if o.most_depended_on.is_empty() {
                                ().into_any()
                            } else {
                                view! {
                                    <h3>"Most depended-on files"</h3>
                                    <table>
                                        <thead><tr><th>"File"</th><th>"Dependents"</th></tr></thead>
                                        <tbody>
                                            {o.most_depended_on.into_iter().map(|(file, count)| view! {
                                                <tr>
                                                    <td>{file_cell(file, selected)}</td>
                                                    <td>{count}</td>
                                                </tr>
                                            }).collect::<Vec<_>>()}
                                        </tbody>
                                    </table>
                                }
                                .into_any()
                            }}
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

#[component]
fn HealthSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let health = LocalResource::new(|| fetch_json::<Health>("/api/health"));

    view! {
        <h2>"Code health"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                health
                    .get()
                    .map(|result| match result.take() {
                        Ok(h) => view! {
                            <p>
                                {format!(
                                    "Average score: {:.1}/10 across {} file(s), {} marker(s) triggered.",
                                    h.average_score, h.file_count, h.finding_count,
                                )}
                            </p>
                            <table>
                                <thead><tr><th>"Marker"</th><th>"Count"</th></tr></thead>
                                <tbody>
                                    {h.by_kind.into_iter().map(|k| view! {
                                        <tr><td>{k.kind}</td><td>{k.count}</td></tr>
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                            <h3>"Lowest-scoring files"</h3>
                            {if h.worst_files.is_empty() {
                                view! { <p class="empty">"No health findings."</p> }.into_any()
                            } else {
                                view! {
                                    <table>
                                        <thead><tr><th>"File"</th><th>"Score"</th><th>"Markers"</th></tr></thead>
                                        <tbody>
                                            {h.worst_files.into_iter().map(|f| view! {
                                                <tr>
                                                    <td>{file_cell(f.file, selected)}</td>
                                                    <td>{format!("{:.1}", f.score)}</td>
                                                    <td>{f.finding_count}</td>
                                                </tr>
                                            }).collect::<Vec<_>>()}
                                        </tbody>
                                    </table>
                                }
                                .into_any()
                            }}
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Symbol detail (issue #263), addressed by `#/symbols?id=<file>@<line>`.
#[component]
fn SymbolDetailSection(id: String, selected: RwSignal<Option<String>>) -> impl IntoView {
    let (file, line) = match id.rsplit_once('@') {
        Some((f, l)) => (f.to_string(), l.to_string()),
        None => (id.clone(), "0".to_string()),
    };
    let detail = LocalResource::new(move || {
        let (f, l) = (file.clone(), line.clone());
        async move {
            fetch_json_with_query::<SymbolDetail>("/api/symbol", &[("file", &f), ("line", &l)])
                .await
        }
    });

    view! {
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                detail
                    .get()
                    .map(|result| match result.take() {
                        Ok(d) if !d.found => view! {
                            <h2>"Symbol not found"</h2>
                            <p class="empty">
                                "No indexed symbol at that location. The link may be stale, or \
                                 the index may predate the file's current shape."
                            </p>
                            <p><a href="#/symbols">"Back to symbols"</a></p>
                        }
                        .into_any(),
                        Ok(d) => view! {
                            <h2>{d.name.clone()}</h2>
                            <p>
                                {format!(
                                    "{}{} — {}:{}-{}",
                                    d.parent.as_ref().map(|p| format!("{p}::")).unwrap_or_default(),
                                    d.kind, d.file, d.start_line, d.end_line,
                                )}
                            </p>
                            <p>
                                {format!(
                                    "Complexity {}, max nesting depth {}.",
                                    d.complexity, d.max_nesting_depth,
                                )}
                            </p>
                            <p>{file_cell(d.file.clone(), selected)}</p>
                            <h3>{format!("Calls ({})", d.callees.len())}</h3>
                            {if d.callees.is_empty() {
                                view! { <p class="empty">"No resolved calls out of this symbol."</p> }
                                    .into_any()
                            } else {
                                view! {
                                    <ul>{d.callees.iter().map(|c| view! { <li class="mono">{c.clone()}</li> })
                                        .collect::<Vec<_>>()}</ul>
                                }
                                .into_any()
                            }}
                            // An empty callee list must not read as "calls
                            // nothing" when resolution simply failed.
                            {(d.unresolved_callee_count > 0)
                                .then(|| view! {
                                    <p class="empty">
                                        {format!(
                                            "{} further call(s) could not be resolved to an \
                                             indexed symbol, so they aren't listed.",
                                            d.unresolved_callee_count,
                                        )}
                                    </p>
                                })}
                            <h3>{format!("Called by ({})", d.callers.len())}</h3>
                            {if d.callers.is_empty() {
                                view! {
                                    <p class="empty">
                                        "No resolved in-repo callers. This port's call resolution \
                                         is heuristic, so that is not proof it's unused."
                                    </p>
                                }
                                .into_any()
                            } else {
                                view! {
                                    <ul>{d.callers.iter().map(|c| view! { <li class="mono">{c.clone()}</li> })
                                        .collect::<Vec<_>>()}</ul>
                                }
                                .into_any()
                            }}
                            <p><a href="#/symbols">"Back to symbols"</a></p>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Decision detail (issue #263), addressed by `#/decisions?id=<id>`.
#[component]
fn DecisionDetailSection(id: String, selected: RwSignal<Option<String>>) -> impl IntoView {
    let detail = LocalResource::new(move || {
        let id = id.clone();
        async move { fetch_json_with_query::<DecisionDetail>("/api/decision", &[("id", &id)]).await }
    });

    view! {
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                detail
                    .get()
                    .map(|result| match result.take() {
                        Ok(d) if !d.found => view! {
                            <h2>"Decision not found"</h2>
                            <p class="empty">"No decision with that id. The link may be stale."</p>
                            <p><a href="#/decisions">"Back to decisions"</a></p>
                        }
                        .into_any(),
                        Ok(d) => view! {
                            <h2>{format!("{} — {}", d.id, d.title)}</h2>
                            // Loudest thing on the page when set: showing a
                            // superseded decision without saying so reads as
                            // current guidance.
                            {d.superseded_by.clone().map(|by| view! {
                                <p class="error">
                                    {format!("SUPERSEDED by {by}. Do not treat this as current.")}
                                    " "
                                    <a href=format_detail_hash(Route::Decisions, &by)>
                                        "Open the superseding decision"
                                    </a>
                                </p>
                            })}
                            {d.supersedes.clone().map(|prev| view! {
                                <p class="empty">
                                    {format!("Supersedes {prev}. ")}
                                    <a href=format_detail_hash(Route::Decisions, &prev)>
                                        "Open the superseded decision"
                                    </a>
                                </p>
                            })}
                            // Second-loudest, and for the same reason as
                            // the supersession banner: this is the page
                            // where someone decides whether to act on a
                            // decision, so "a model guessed this" cannot
                            // be a detail they have to go looking for.
                            {d.inferred.then(|| view! {
                                <p class="error">
                                    "INFERRED BY A MODEL from code, not read from an ADR, \
                                     commit, or comment. It is anchored to code the model \
                                     quoted -- a quote that no longer appears in the file \
                                     drops the decision -- but it is a reading of the code, \
                                     not recorded intent."
                                </p>
                            })}
                            {d.status.clone().map(|st| view! { <p>{format!("Status: {st}")}</p> })}
                            <p class="empty">{format!("Source: {}", d.source)}</p>
                            <h3>{format!("Linked files ({})", d.linked_files.len())}</h3>
                            {if d.linked_files.is_empty() {
                                view! { <p class="empty">"No linked files."</p> }.into_any()
                            } else {
                                view! {
                                    <ul>
                                        {d.linked_files.iter().map(|f| view! {
                                            <li>{file_cell(f.clone(), selected)}</li>
                                        }).collect::<Vec<_>>()}
                                    </ul>
                                }
                                .into_any()
                            }}
                            <h3>"Text"</h3>
                            <pre class="mono">{d.body.clone()}</pre>
                            <p><a href="#/decisions">"Back to decisions"</a></p>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Commit activity (issue #262): a day×hour punch card and a weekly
/// trend, both as inline SVG.
///
/// Both charts are drawn by hand rather than via a charting crate, for
/// the same reason as the treemap: a WASM binary shouldn't grow one for
/// a grid of rects and a polyline.
#[component]
fn StatsSection() -> impl IntoView {
    let stats = LocalResource::new(|| fetch_json::<Stats>("/api/stats"));
    const CELL: f64 = 22.0;
    const LEFT: f64 = 40.0;
    const TOP: f64 = 18.0;

    view! {
        <h2>"Activity"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                stats
                    .get()
                    .map(|result| match result.take() {
                        Ok(s) if !s.available => view! {
                            <p class="empty">
                                "No commit history -- needs a git repository with commits."
                            </p>
                        }
                        .into_any(),
                        Ok(s) => {
                            let peak = s
                                .punch_card
                                .iter()
                                .flatten()
                                .copied()
                                .max()
                                .unwrap_or(0)
                                .max(1);
                            let trend_max = s.weekly_trend.iter().copied().max().unwrap_or(0).max(1);
                            let tw = 640.0f64;
                            let th = 120.0f64;
                            let step = if s.weekly_trend.len() > 1 {
                                tw / (s.weekly_trend.len() - 1) as f64
                            } else {
                                tw
                            };
                            let points = s
                                .weekly_trend
                                .iter()
                                .enumerate()
                                .map(|(i, c)| {
                                    let x = i as f64 * step;
                                    let y = th - (*c as f64 / trend_max as f64) * th;
                                    format!("{x:.1},{y:.1}")
                                })
                                .collect::<Vec<_>>()
                                .join(" ");
                            view! {
                                <p>
                                    {format!(
                                        "{} commit(s). All times {} -- git stores an author \
                                         timezone offset this port does not carry, so bucketing \
                                         in anything else would be guesswork.",
                                        s.commit_count, s.timezone,
                                    )}
                                </p>
                                // A shallow clone doesn't make these charts
                                // fail, it makes them under-report. Saying so
                                // is the difference between a caveat and a
                                // wrong answer.
                                {s.shallow
                                    .then(|| view! {
                                        <p class="empty">
                                            "Shallow clone -- history is truncated, so both \
                                             charts under-report. `git fetch --unshallow` for \
                                             the full picture."
                                        </p>
                                    })}
                                <h3>{format!("Punch card ({})", s.timezone)}</h3>
                                <svg
                                    viewBox=format!("0 0 {} {}", LEFT + 24.0 * CELL, TOP + 7.0 * CELL)
                                    style="width: 100%; height: auto;"
                                    role="img"
                                >
                                    {(0..24).step_by(3).map(|h| view! {
                                        <text
                                            x=LEFT + h as f64 * CELL
                                            y=TOP - 5.0
                                            font-size="10"
                                            fill="currentColor"
                                        >{format!("{h:02}")}</text>
                                    }).collect::<Vec<_>>()}
                                    {s.punch_card.iter().enumerate().map(|(d, hours)| {
                                        let row = hours.iter().enumerate().map(|(h, count)| {
                                            // Opacity carries magnitude; the
                                            // <title> carries the number, so
                                            // the value is never colour-only.
                                            let o = *count as f64 / peak as f64;
                                            view! {
                                                <g>
                                                    <title>
                                                        {format!(
                                                            "{} {:02}:00 {} -- {} commit(s)",
                                                            DAY_LABELS[d], h, s.timezone, count,
                                                        )}
                                                    </title>
                                                    <rect
                                                        x=LEFT + h as f64 * CELL
                                                        y=TOP + d as f64 * CELL
                                                        width=CELL - 2.0
                                                        height=CELL - 2.0
                                                        fill="#1565c0"
                                                        fill-opacity=format!("{:.3}", 0.08 + o * 0.92)
                                                    />
                                                </g>
                                            }
                                        }).collect::<Vec<_>>();
                                        view! {
                                            <g>
                                                <text
                                                    x="0"
                                                    y=TOP + d as f64 * CELL + CELL / 1.6
                                                    font-size="10"
                                                    fill="currentColor"
                                                >{DAY_LABELS[d]}</text>
                                                {row}
                                            </g>
                                        }
                                    }).collect::<Vec<_>>()}
                                </svg>
                                <h3>{format!("Weekly commits (last {} weeks)", s.weekly_trend.len())}</h3>
                                <svg
                                    viewBox=format!("0 0 {tw} {th}")
                                    style="width: 100%; height: auto;"
                                    role="img"
                                >
                                    <title>
                                        {format!("Peak {trend_max} commit(s) in a week")}
                                    </title>
                                    <polyline
                                        points=points
                                        fill="none"
                                        stroke="#1565c0"
                                        stroke-width="2"
                                    />
                                </svg>
                            }
                            .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Files treemap (issue #261): area proportional to lines, fill by
/// health band.
///
/// Answers what the ranked tables cannot: where the mass of the codebase
/// sits, and whether the big parts are healthy. A "10 worst files" table
/// hides a large mediocre file behind ten small terrible ones.
#[component]
fn FilesSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let files = LocalResource::new(|| fetch_json::<Files>("/api/files"));
    const W: f64 = 900.0;
    const H: f64 = 420.0;

    view! {
        <h2>"Files"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                files
                    .get()
                    .map(|result| match result.take() {
                        Ok(f) if f.files.is_empty() => view! {
                            <p class="empty">"No indexed files."</p>
                        }
                        .into_any(),
                        Ok(f) => {
                            let values: Vec<f64> = f.files.iter().map(|e| e.lines as f64).collect();
                            let tiles = squarify(&values, W, H);
                            let entries = f.files.clone();
                            let total_lines = f.total_lines;
                            let health_available = f.health_available;
                            view! {
                                <p>
                                    {format!(
                                        "{} file(s), {} total line(s). Area is proportional to \
                                         lines; fill is the health band.",
                                        entries.len(), total_lines,
                                    )}
                                </p>
                                // Degrade honestly rather than rendering
                                // every tile grey with no explanation.
                                {(!health_available)
                                    .then(|| view! {
                                        <p class="empty">
                                            "Health scoring unavailable -- tiles are sized but \
                                             not colored."
                                        </p>
                                    })}
                                <p class="empty">
                                    "Bands: good (>=8), fair (>=5), poor (<5), unscored. \
                                     Each tile names its band on hover, so color is not the \
                                     only channel."
                                </p>
                                <svg
                                    viewBox=format!("0 0 {W} {H}")
                                    style="width: 100%; height: auto; border: 1px solid #8884;"
                                    role="img"
                                >
                                    {tiles.into_iter().filter_map(|t| {
                                        let e = entries.get(t.index)?.clone();
                                        let (band, fill) = health_band(e.score);
                                        let label = match e.score {
                                            Some(sc) => format!(
                                                "{} -- {} lines, {} ({:.1}/10), {} marker(s)",
                                                e.path, e.lines, band, sc, e.finding_count,
                                            ),
                                            None => format!(
                                                "{} -- {} lines, {} ({})",
                                                e.path, e.lines, band, e.language,
                                            ),
                                        };
                                        let path = e.path.clone();
                                        Some(view! {
                                            <g>
                                                <title>{label}</title>
                                                <rect
                                                    x=t.x y=t.y width=t.w height=t.h
                                                    fill=fill
                                                    stroke="#fff"
                                                    stroke-width="1"
                                                    style="cursor: pointer;"
                                                    on:click=move |_| selected.set(Some(path.clone()))
                                                />
                                            </g>
                                        })
                                    }).collect::<Vec<_>>()}
                                </svg>
                            }
                            .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Contributor directory and bus-factor distribution (issue #258).
///
/// Bus factor is rendered in words, not as a bare number: "1" reads to
/// some as "one clear owner, tidy" -- the opposite of its meaning. The
/// CLI's `repowise ownership` spells it out for the same reason, and
/// this must not regress that.
#[component]
fn ContributorsSection() -> impl IntoView {
    let contributors = LocalResource::new(|| fetch_json::<Contributors>("/api/contributors"));

    view! {
        <h2>"Contributors"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                contributors
                    .get()
                    .map(|result| match result.take() {
                        Ok(c) if !c.available => view! {
                            <p class="empty">
                                "No ownership data -- needs a git repository with history."
                            </p>
                        }
                        .into_any(),
                        Ok(c) => view! {
                            // Say when the sweep was bounded. Otherwise the
                            // shares read as repo-wide when they aren't.
                            <p>
                                {format!(
                                    "Ownership across {} of {} indexed file(s).",
                                    c.files_sampled, c.files_total,
                                )}
                            </p>
                            // Two different reasons the sample can fall
                            // short of the repo, reported separately --
                            // "bounded sample" on a repo where the bound
                            // never applied would be plain wrong.
                            {c.limit_applied
                                .then(|| view! {
                                    <p class="empty">
                                        "Bounded to the largest files -- this is a sample, not \
                                         the whole repo."
                                    </p>
                                })}
                            {(c.files_unblameable > 0)
                                .then(|| view! {
                                    <p class="empty">
                                        {format!(
                                            "{} file(s) could not be blamed (untracked or never \
                                             committed) and contributed nothing.",
                                            c.files_unblameable,
                                        )}
                                    </p>
                                })}
                            <table>
                                <thead>
                                    <tr>
                                        <th>"Author"</th>
                                        <th>"Share"</th>
                                        <th>"Lines"</th>
                                        <th>"Files"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {c.contributors.into_iter().map(|a| view! {
                                        <tr>
                                            <td>{a.author}</td>
                                            <td>{format!("{:.1}%", a.percent)}</td>
                                            <td>{a.lines_owned}</td>
                                            <td>{a.files_touched}</td>
                                        </tr>
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                            <h3>"Bus factor"</h3>
                            <p class="empty">
                                "How many authors would have to leave before most of a file has \
                                 no author left who has touched it. Lower is riskier."
                            </p>
                            <table>
                                <thead><tr><th>"Bus factor"</th><th>"Files"</th></tr></thead>
                                <tbody>
                                    {c.bus_factor_distribution.into_iter().map(|(bf, count)| {
                                        let label = match bf {
                                            1 => "1 -- one author wrote most of the file".to_string(),
                                            n => format!("{n} -- {n} authors between them"),
                                        };
                                        view! { <tr><td>{label}</td><td>{count}</td></tr> }
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Test coverage (issue #257), surfacing what `repowise coverage add`
/// ingested.
///
/// The distinction this view exists to preserve: a file that no report
/// measured is **not** a file at 0%. The server keeps them apart
/// (measured files in `files`, the rest counted in `unmeasured_files`)
/// and this renders both, so an unmeasured repo can never look like an
/// untested one.
#[component]
fn CoverageSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let coverage = LocalResource::new(|| fetch_json::<Coverage>("/api/coverage"));

    view! {
        <h2>"Test coverage"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                coverage
                    .get()
                    .map(|result| match result.take() {
                        Ok(c) if !c.available => view! {
                            <p class="empty">
                                "No coverage ingested. Run `repowise coverage add <REPORT>` \
                                 to populate this view."
                            </p>
                        }
                        .into_any(),
                        Ok(c) => view! {
                            <p>
                                {format!(
                                    "{:.1}% mean line coverage across {} measured file(s).",
                                    c.mean_percent,
                                    c.files.len(),
                                )}
                            </p>
                            // Stated, never implied: without this the measured
                            // set reads as the whole repo.
                            {(c.unmeasured_files > 0)
                                .then(|| view! {
                                    <p class="empty">
                                        {format!(
                                            "{} indexed file(s) appear in no report -- unmeasured, \
                                             which is not the same as untested.",
                                            c.unmeasured_files,
                                        )}
                                    </p>
                                })}
                            <p>
                                {if c.has_per_test_map {
                                    format!(
                                        "Per-test map: {} test context(s) -- `repowise \
                                         impacted-tests` can run.",
                                        c.test_contexts,
                                    )
                                } else {
                                    "Per-test map: none. The ingested reports carried no TN: \
                                     records, so `repowise impacted-tests` cannot run."
                                        .to_string()
                                }}
                            </p>
                            <h3>"Least-covered files"</h3>
                            <table>
                                <thead>
                                    <tr>
                                        <th>"File"</th>
                                        <th>"Coverage"</th>
                                        <th>"Lines hit"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {c.files.into_iter().map(|f| view! {
                                        <tr>
                                            <td>{file_cell(f.path, selected)}</td>
                                            <td>{format!("{:.0}%", f.percent)}</td>
                                            <td>{format!("{} / {}", f.lines_hit, f.lines_known)}</td>
                                        </tr>
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

#[component]
fn HotspotsSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let hotspots = LocalResource::new(|| fetch_json::<Hotspots>("/api/hotspots"));

    view! {
        <h2>"Hotspots"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                hotspots
                    .get()
                    .map(|result| match result.take() {
                        Ok(h) if !h.available => {
                            view! { <p class="empty">"No git history found under this root."</p> }.into_any()
                        }
                        Ok(h) if h.hotspots.is_empty() => {
                            view! { <p class="empty">"No file has both git history and complexity."</p> }.into_any()
                        }
                        Ok(h) => view! {
                            <table>
                                <thead>
                                    <tr>
                                        <th>"File"</th>
                                        <th>"Score (recency-weighted)"</th>
                                        <th>"Score (raw)"</th>
                                        <th>"Churn"</th>
                                        <th>"Complexity"</th>
                                        <th>"Bugfixes"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {h.hotspots.into_iter().map(|hs| view! {
                                        <tr>
                                            <td>{file_cell(hs.file, selected)}</td>
                                            <td>{format!("{:.1}", hs.decayed_score)}</td>
                                            <td>{hs.score}</td>
                                            <td>{hs.churn}</td>
                                            <td>{hs.total_complexity}</td>
                                            <td>{hs.bugfix_commits}</td>
                                        </tr>
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

#[component]
fn DecisionsSection() -> impl IntoView {
    let decisions = LocalResource::new(|| fetch_json::<Decisions>("/api/decisions"));

    view! {
        <h2>"Architectural decisions"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                decisions
                    .get()
                    .map(|result| match result.take() {
                        Ok(ds) if ds.decisions.is_empty() => view! {
                            <p class="empty">
                                "No decisions found (docs/adr/*.md or decision-like commits)."
                            </p>
                            <p class="empty">{ds.inferred_source}</p>
                        }
                        .into_any(),
                        Ok(ds) => view! {
                            <p class="empty">{ds.inferred_source.clone()}</p>
                            <table>
                                <thead>
                                    <tr>
                                        <th>"ID"</th>
                                        <th>"Title"</th>
                                        <th>"Status"</th>
                                        <th>"Source"</th>
                                        <th>"Linked files"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {ds.decisions.into_iter().map(|d| {
                                        // Only ADR files carry a `Status:`
                                        // line. The absent case used to
                                        // render as "commit", from when
                                        // commits were the only other
                                        // source -- which now mislabels
                                        // comments, changelog entries and
                                        // LLM inferences. The Source column
                                        // says where it came from.
                                        let status = match d.superseded_by {
                                            Some(target) => format!("superseded by {target}"),
                                            None => d.status.unwrap_or_default(),
                                        };
                                        // The badge is the whole point of
                                        // carrying `inferred` to the client.
                                        let badge = d.inferred.then(|| view! {
                                            <span class="badge">"inferred"</span>" "
                                        });
                                        view! {
                                            <tr>
                                                <td>
                                                    <a href=format_detail_hash(
                                                        Route::Decisions, &d.id,
                                                    )>{d.id.clone()}</a>
                                                </td>
                                                <td>{badge}{d.title}</td>
                                                <td>{status}</td>
                                                <td>{d.source}</td>
                                                <td>{d.linked_file_count}</td>
                                            </tr>
                                        }
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

#[component]
fn SymbolsSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let symbols = LocalResource::new(|| fetch_json::<Vec<Symbol>>("/api/symbols"));
    let filter = RwSignal::new(String::new());

    view! {
        <h2>"Symbols"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                symbols
                    .get()
                    .map(|result| match result.take() {
                        Ok(syms) if syms.is_empty() => {
                            view! { <p class="empty">"No symbols indexed."</p> }.into_any()
                        }
                        Ok(syms) => {
                            let mut kinds: Vec<String> = syms.iter().map(|s| s.kind.clone()).collect();
                            kinds.sort();
                            kinds.dedup();
                            let count = syms.len();
                            view! {
                                <p>{format!("{count} symbol(s).")}</p>
                                <label for="symbol-kind-filter">"Filter by kind: "</label>
                                <select
                                    id="symbol-kind-filter"
                                    on:change=move |ev| filter.set(event_target_value(&ev))
                                >
                                    <option value="">"All"</option>
                                    {kinds.into_iter().map(|k| {
                                        let label = k.clone();
                                        view! { <option value=k>{label}</option> }
                                    }).collect::<Vec<_>>()}
                                </select>
                                <table>
                                    <thead>
                                        <tr><th>"Name"</th><th>"Kind"</th><th>"File"</th><th>"Line"</th></tr>
                                    </thead>
                                    <tbody>
                                        {move || {
                                            let active = filter.get();
                                            syms.iter()
                                                .filter(|s| active.is_empty() || s.kind == active)
                                                .map(|s| view! {
                                                    <tr>
                                                        <td>
                                                            <a href=format_detail_hash(
                                                                Route::Symbols,
                                                                &format!("{}@{}", s.file, s.start_line),
                                                            )>{s.name.clone()}</a>
                                                        </td>
                                                        <td>{s.kind.clone()}</td>
                                                        <td>{file_cell(s.file.clone(), selected)}</td>
                                                        <td>{s.start_line}</td>
                                                    </tr>
                                                })
                                                .collect::<Vec<_>>()
                                        }}
                                    </tbody>
                                </table>
                            }
                            .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// One file's full drill-down: its wiki page (if `repowise docs` has
/// already generated one), git-blame ownership breakdown, and any
/// architectural decisions linked to it -- each loads and fails
/// independently, so a file with no wiki page yet still shows whatever
/// ownership/decision data is available instead of one shared error.
#[component]
fn FileDetail(path: String, has_wiki: bool, selected: RwSignal<Option<String>>) -> impl IntoView {
    let wiki = LocalResource::new({
        let path = path.clone();
        move || {
            let path = path.clone();
            async move {
                if has_wiki {
                    fetch_json_with_query::<WikiPage>("/api/wiki", &[("path", &path)])
                        .await
                        .map(Some)
                } else {
                    Ok(None)
                }
            }
        }
    });
    let ownership = LocalResource::new({
        let path = path.clone();
        move || {
            let path = path.clone();
            async move { fetch_json_with_query::<Ownership>("/api/ownership", &[("path", &path)]).await }
        }
    });
    let decisions = LocalResource::new({
        let path = path.clone();
        move || {
            let path = path.clone();
            async move { fetch_json_with_query::<Decisions>("/api/decisions", &[("file", &path)]).await }
        }
    });
    let title = path.clone();

    view! {
        <div class="file-detail">
            <div class="file-detail-header">
                <strong>{title}</strong>
                <button on:click=move |_| selected.set(None)>"Close"</button>
            </div>

            <h3>"Wiki"</h3>
            <Suspense fallback=|| view! { <p>"Loading..."</p> }>
                {move || {
                    wiki.get().map(|result| match result.take() {
                        Ok(Some(w)) => view! { <pre>{w.content}</pre> }.into_any(),
                        Ok(None) => {
                            view! { <p class="empty">"No wiki page yet -- run `repowise docs`."</p> }
                                .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
                }}
            </Suspense>

            <h3>"Ownership"</h3>
            <Suspense fallback=|| view! { <p>"Loading..."</p> }>
                {move || {
                    ownership
                        .get()
                        .map(|result| match result.take() {
                            Ok(o) if !o.available => {
                                view! { <p class="empty">"No git history found for this file."</p> }
                                    .into_any()
                            }
                            Ok(o) => view! {
                                <table>
                                    <thead>
                                        <tr><th>"Author"</th><th>"Lines"</th><th>"Share"</th></tr>
                                    </thead>
                                    <tbody>
                                        {o.owners.into_iter().map(|owner| view! {
                                            <tr>
                                                <td>{owner.author}</td>
                                                <td>{owner.lines}</td>
                                                <td>{format!("{:.1}%", owner.percentage)}</td>
                                            </tr>
                                        }).collect::<Vec<_>>()}
                                    </tbody>
                                </table>
                            }
                            .into_any(),
                            Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                        })
                }}
            </Suspense>

            <h3>"Linked decisions"</h3>
            <Suspense fallback=|| view! { <p>"Loading..."</p> }>
                {move || {
                    decisions
                        .get()
                        .map(|result| match result.take() {
                            Ok(ds) if ds.decisions.is_empty() => {
                                view! {
                                    <p class="empty">"No decisions linked to this file."</p>
                                    <p class="empty">{ds.inferred_source}</p>
                                }
                                    .into_any()
                            }
                            Ok(ds) => view! {
                                <ul>
                                    {ds.decisions.into_iter().map(|d| {
                                        let badge = d.inferred.then(|| view! {
                                            <span class="badge">"inferred"</span>" "
                                        });
                                        view! {
                                            <li>{badge}<strong>{d.id}</strong>": "{d.title}</li>
                                        }
                                    }).collect::<Vec<_>>()}
                                </ul>
                            }
                            .into_any(),
                            Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                        })
                }}
            </Suspense>
        </div>
    }
}

#[component]
fn FileDetailPanel(wiki_pages: WikiPages, selected: RwSignal<Option<String>>) -> impl IntoView {
    view! {
        {move || {
            selected.get().map(|path| {
                let has_wiki = wiki_pages_snapshot(wiki_pages).contains(&path);
                view! { <FileDetail path=path has_wiki=has_wiki selected=selected /> }
            })
        }}
    }
}

/// Current location hash, or empty when unavailable.
fn location_hash() -> String {
    web_sys::window()
        .and_then(|w| w.location().hash().ok())
        .unwrap_or_default()
}

/// Update the hash without pushing a history entry.
///
/// `replaceState` rather than assigning `location.hash`: selecting a
/// file shouldn't add a back-button step for every click, matching what
/// present mode already does for slides.
fn replace_hash(hash: &str) {
    if let Some(history) = web_sys::window().and_then(|w| w.history().ok()) {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(hash));
    }
}

/// One view. Every section that used to render stacked on a single
/// page is now reachable at its own route (issue #259).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    Overview,
    Health,
    Coverage,
    Hotspots,
    Contributors,
    Files,
    Stats,
    Decisions,
    Symbols,
    Graph,
    DeadCode,
    RefactorCandidates,
    Docs,
    Coupling,
    ExternalDeps,
    Commits,
    Communities,
    Chat,
    Usage,
    Settings,
    Workspace,
    CoChanges,
    SystemMap,
    Conformance,
    Contracts,
    /// A hash that matches no known view. Rendered as an explicit
    /// not-found state rather than a blank page or a silent redirect --
    /// a stale bookmark should say so.
    NotFound,
}

/// Every routable view, in nav order, as `(route, slug, label)`.
///
/// Single source of truth: the nav, the parser, and the formatter all
/// read this table, so a view can't be added to one and forgotten in
/// another.
const ROUTES: &[(Route, &str, &str)] = &[
    (Route::Overview, "overview", "Overview"),
    (Route::Health, "health", "Code health"),
    (Route::Coverage, "coverage", "Coverage"),
    (Route::Hotspots, "hotspots", "Hotspots"),
    (Route::Contributors, "contributors", "Contributors"),
    (Route::Files, "files", "Files"),
    (Route::Stats, "stats", "Activity"),
    (Route::Decisions, "decisions", "Decisions"),
    (Route::Symbols, "symbols", "Symbols"),
    (Route::Graph, "graph", "Graph"),
    (Route::DeadCode, "dead-code", "Dead code"),
    (
        Route::RefactorCandidates,
        "refactor-candidates",
        "Refactoring",
    ),
    (Route::Docs, "docs", "Docs"),
    (Route::Coupling, "coupling", "Coupling"),
    (Route::ExternalDeps, "dependencies", "Dependencies"),
    (Route::Commits, "commits", "Commits"),
    (Route::Communities, "map", "Map"),
    (Route::Chat, "chat", "Chat"),
    (Route::Usage, "usage", "Usage"),
    (Route::Settings, "settings", "Settings"),
    (Route::Workspace, "workspace", "Workspace"),
    (Route::CoChanges, "co-changes", "Co-changes"),
    (Route::SystemMap, "system-map", "System map"),
    (Route::Conformance, "conformance", "Conformance"),
    (Route::Contracts, "contracts", "Contracts"),
];

fn route_slug(route: Route) -> &'static str {
    ROUTES
        .iter()
        .find(|(r, _, _)| *r == route)
        .map(|(_, slug, _)| *slug)
        .unwrap_or("overview")
}

/// Parse a location hash into a view and an optional selected file.
///
/// Shape: `#/<slug>` with an optional `?file=<percent-encoded path>`.
///
/// **Hash routing, not path routing**, and deliberately: `repowise
/// serve-dashboard` serves static files, so `/health` would 404 on
/// reload unless the server grew a catch-all rewrite to `index.html`.
/// A hash survives a reload with no server change at all, and the app
/// already used a hash for present mode -- so this follows the existing
/// convention rather than introducing a second one.
///
/// Pure, so every case below is testable without a browser.
fn parse_hash(hash: &str) -> (Route, Option<String>) {
    let (route, sel, _) = parse_hash_full(hash);
    (route, sel)
}

/// As [`parse_hash`], plus the `id` parameter used by the detail views
/// (issue #263) to address one symbol or decision.
fn parse_hash_full(hash: &str) -> (Route, Option<String>, Option<String>) {
    let raw = hash.trim().trim_start_matches('#');
    // Present mode owns `#present/<n>` and is an overlay, not a view;
    // leave the underlying route alone so exiting returns you to it.
    if raw.is_empty() || raw.starts_with("present/") {
        return (Route::Overview, None, None);
    }
    let raw = raw.trim_start_matches('/');
    let (slug, query) = match raw.split_once('?') {
        Some((s, q)) => (s, Some(q)),
        None => (raw, None),
    };
    if slug.is_empty() {
        return (Route::Overview, None, None);
    }

    let route = ROUTES
        .iter()
        .find(|(_, s, _)| *s == slug)
        .map(|(r, _, _)| *r)
        .unwrap_or(Route::NotFound);

    let param = |key: &str| {
        query.and_then(|q| {
            q.split('&')
                .find_map(|pair| pair.strip_prefix(key))
                .map(percent_decode)
                .filter(|v| !v.is_empty())
        })
    };
    (route, param("file="), param("id="))
}

/// Format a route with an addressed detail `id` (issue #263).
fn format_detail_hash(route: Route, id: &str) -> String {
    format!("#/{}?id={}", route_slug(route), percent_encode(id))
}

/// Format a view and selection back into a hash.
fn format_hash(route: Route, selected: Option<&str>) -> String {
    let mut out = format!("#/{}", route_slug(route));
    if let Some(file) = selected.filter(|f| !f.is_empty()) {
        out.push_str("?file=");
        out.push_str(&percent_encode(file));
    }
    out
}

/// Minimal percent-encoding for the characters that would otherwise
/// break a hash query: `%`, `&`, `#`, `?`, `+`, and space.
///
/// Hand-rolled rather than adding a dependency for six characters.
/// Paths are the only thing encoded here and they don't contain the
/// wider set a general URL encoder would need to handle.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'%' => out.push_str("%25"),
            b'&' => out.push_str("%26"),
            b'#' => out.push_str("%23"),
            b'?' => out.push_str("%3F"),
            b'+' => out.push_str("%2B"),
            b' ' => out.push_str("%20"),
            _ => out.push(b as char),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// How long typing must pause before a search request is issued.
///
/// Without this every keystroke was one HTTP request: typing "parser"
/// fired six, five of which were obsolete before they returned. 200ms is
/// short enough to feel instant and long enough to collapse a burst of
/// typing into a single request.
const SEARCH_DEBOUNCE_MS: u32 = 200;

/// Enforced at compile time rather than in a test: the constraint is on
/// the constant itself, so a future "fix" that raises it into
/// perceptible-lag territory should fail the build, not a test run.
const _: () = assert!(SEARCH_DEBOUNCE_MS > 0 && SEARCH_DEBOUNCE_MS <= 300);

/// Whether a query is worth sending to the server.
///
/// Guards the "empty query" case explicitly: `/api/search` returns
/// nothing for an empty needle anyway, so issuing the request would be
/// pure waste, and rendering its empty result would look like "no
/// matches" rather than "you haven't typed anything".
fn should_query(q: &str) -> bool {
    !q.trim().is_empty()
}

/// A Ctrl/Cmd+K instant search box over files and symbols. Results are
/// live-fetched from `/api/search` as you type; clicking a file result
/// opens its file-detail panel the same way a drill-down link does.
#[component]
fn SearchBox(selected: RwSignal<Option<String>>) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let input_ref: NodeRef<html::Input> = NodeRef::new();

    window_event_listener_untyped("keydown", move |ev| {
        if let Some(kb) = ev.dyn_ref::<web_sys::KeyboardEvent>() {
            if (kb.meta_key() || kb.ctrl_key()) && kb.key().eq_ignore_ascii_case("k") {
                kb.prevent_default();
                if let Some(el) = input_ref.get() {
                    let _ = el.focus();
                }
            }
        }
    });

    let results = LocalResource::new(move || {
        let q = query.get();
        async move {
            if !should_query(&q) {
                return Ok(SearchResults {
                    files: Vec::new(),
                    symbols: Vec::new(),
                });
            }
            // Debounce. A further keystroke re-runs this resource and
            // drops the in-flight future before the delay elapses, so
            // only a pause in typing actually issues a request.
            gloo_timers::future::TimeoutFuture::new(SEARCH_DEBOUNCE_MS).await;
            fetch_json_with_query::<SearchResults>("/api/search", &[("q", &q)]).await
        }
    });

    view! {
        <div class="search-box">
            <input
                type="search"
                placeholder="Search files and symbols... (Ctrl/Cmd+K)"
                node_ref=input_ref
                prop:value=move || query.get()
                on:input=move |ev| query.set(event_target_value(&ev))
            />
            <Suspense fallback=|| ()>
                {move || {
                    // A prompt, not silence -- an empty panel reads as
                    // "no matches" when nothing has been typed yet.
                    if !should_query(&query.get()) {
                        return Some(
                            view! {
                                <p class="empty">
                                    "Type to search files and symbols."
                                </p>
                            }
                            .into_any(),
                        );
                    }
                    results.get().map(|result| match result.take() {
                        Ok(res) if res.files.is_empty() && res.symbols.is_empty() => {
                            view! { <p class="empty">"No matches."</p> }.into_any()
                        }
                        Ok(res) => view! {
                            <p class="empty">
                                {format!(
                                    "{} file(s), {} symbol(s).",
                                    res.files.len(), res.symbols.len(),
                                )}
                            </p>
                            <ul class="search-results">
                                {res.files.into_iter().map(|f| {
                                    let target = f.clone();
                                    view! {
                                        <li>
                                            <a href="#" on:click=move |ev| {
                                                ev.prevent_default();
                                                selected.set(Some(target.clone()));
                                            }>{f}</a>
                                        </li>
                                    }
                                }).collect::<Vec<_>>()}
                                // Symbols were plain text while files were
                                // links; a symbol result you can't act on is
                                // half a result. Clicking selects its file.
                                {res.symbols.into_iter().map(|s| {
                                    let target = s.file.clone();
                                    let label = format!(
                                        "{} ({}) — {}:{}", s.name, s.kind, s.file, s.start_line,
                                    );
                                    view! {
                                        <li class="mono">
                                            <a href="#" on:click=move |ev| {
                                                ev.prevent_default();
                                                selected.set(Some(target.clone()));
                                            }>{label}</a>
                                        </li>
                                    }
                                }).collect::<Vec<_>>()}
                            </ul>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
                }}
            </Suspense>
        </div>
    }
}

const GRAPH_WIDTH: f64 = 720.0;
const GRAPH_HEIGHT: f64 = 480.0;
const GRAPH_LAYOUT_ITERATIONS: usize = 300;

/// A small Fruchterman-Reingold-style force-directed layout: nodes start
/// evenly spaced on a circle (deterministic, no RNG needed), then repel
/// each other, get pulled together along edges like springs, and get a
/// gentle pull back toward the canvas center so the layout doesn't
/// drift off-screen. `n` up to `GRAPH_NODE_LIMIT`-ish keeps this well
/// within budget for a WASM tab -- it's `O(n^2)` per iteration, from the
/// all-pairs repulsion term.
fn layout(nodes: &[GraphNode], edges: &[GraphEdge]) -> Vec<(f64, f64)> {
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let cx = GRAPH_WIDTH / 2.0;
    let cy = GRAPH_HEIGHT / 2.0;
    let radius = (GRAPH_WIDTH.min(GRAPH_HEIGHT) / 2.0) * 0.8;

    let mut pos: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            (cx + radius * angle.cos(), cy + radius * angle.sin())
        })
        .collect();

    let index_of: std::collections::HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.as_str(), i))
        .collect();
    let edge_pairs: Vec<(usize, usize)> = edges
        .iter()
        .filter_map(|e| {
            Some((
                *index_of.get(e.from.as_str())?,
                *index_of.get(e.to.as_str())?,
            ))
        })
        .collect();

    const REPULSION: f64 = 6000.0;
    const SPRING_LENGTH: f64 = 90.0;
    const SPRING_STRENGTH: f64 = 0.02;
    const CENTER_STRENGTH: f64 = 0.01;

    for _ in 0..GRAPH_LAYOUT_ITERATIONS {
        let mut disp = vec![(0.0_f64, 0.0_f64); n];

        for i in 0..n {
            for j in (i + 1)..n {
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let dist_sq = (dx * dx + dy * dy).max(1.0);
                let dist = dist_sq.sqrt();
                let force = REPULSION / dist_sq;
                let (fx, fy) = (force * dx / dist, force * dy / dist);
                disp[i].0 += fx;
                disp[i].1 += fy;
                disp[j].0 -= fx;
                disp[j].1 -= fy;
            }
        }

        for &(a, b) in &edge_pairs {
            let dx = pos[b].0 - pos[a].0;
            let dy = pos[b].1 - pos[a].1;
            let dist = (dx * dx + dy * dy).sqrt().max(1.0);
            let force = SPRING_STRENGTH * (dist - SPRING_LENGTH);
            let (fx, fy) = (force * dx / dist, force * dy / dist);
            disp[a].0 += fx;
            disp[a].1 += fy;
            disp[b].0 -= fx;
            disp[b].1 -= fy;
        }

        for (i, p) in pos.iter().enumerate() {
            disp[i].0 += (cx - p.0) * CENTER_STRENGTH;
            disp[i].1 += (cy - p.1) * CENTER_STRENGTH;
        }

        for (i, p) in pos.iter_mut().enumerate() {
            p.0 = (p.0 + disp[i].0.clamp(-10.0, 10.0)).clamp(20.0, GRAPH_WIDTH - 20.0);
            p.1 = (p.1 + disp[i].1.clamp(-10.0, 10.0)).clamp(20.0, GRAPH_HEIGHT - 20.0);
        }
    }

    pos
}

/// GitHub's own per-language colors, for a graph view that reads at a
/// glance the same way GitHub's language bar does. Falls back to a
/// neutral gray for anything not in this list (e.g. "Other").
fn language_color(language: &str) -> &'static str {
    match language {
        "Rust" => "#dea584",
        "Python" => "#3572A5",
        "TypeScript" => "#3178c6",
        "JavaScript" => "#f1e05a",
        "Java" => "#b07219",
        "Kotlin" => "#A97BFF",
        "Go" => "#00ADD8",
        "C++" => "#f34b7d",
        "C#" => "#178600",
        "Scala" => "#c22d40",
        "Ruby" => "#701516",
        "C" => "#555555",
        "Swift" => "#F05138",
        "PHP" => "#4F5D95",
        "Dart" => "#00B4AB",
        "Shell" => "#89e051",
        "Luau" => "#00A2FF",
        "Objective-C" => "#438eff",
        "R" => "#198CE7",
        "Zig" => "#ec915c",
        "Julia" => "#a270ba",
        "Elm" => "#60B5CC",
        "OCaml" => "#3be133",
        "Crystal" => "#000100",
        "Nim" => "#ffc200",
        "D" => "#ba595e",
        _ => "#767676",
    }
}

fn short_label(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A visual graph of file-level import dependencies -- the last major
/// piece of static-dashboard parity this pivot was missing. Clicking a
/// node opens its wiki page inline, the same drill-down convention
/// every other section uses.
#[component]
fn GraphSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    // Module granularity (issue #354) is a bolted-on toggle over the
    // same file-level import graph, not a separate view: `/api/graph`
    // and `/api/graph-modules` return the identical DTO shape, so the
    // rest of this component (layout, coloring, click-to-select) is
    // reused unchanged regardless of which is fetched.
    let by_module = RwSignal::new(false);
    let graph = LocalResource::new(move || {
        let url = if by_module.get() {
            "/api/graph-modules"
        } else {
            "/api/graph"
        };
        async move { fetch_json::<Graph>(url).await }
    });

    view! {
        <h2>"Dependency graph"</h2>
        <label>
            <input
                type="checkbox"
                prop:checked=move || by_module.get()
                on:change=move |ev| by_module.set(event_target_checked(&ev))
            />
            " Group by module (directory)"
        </label>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                graph
                    .get()
                    .map(|result| match result.take() {
                        Ok(g) if g.nodes.is_empty() => {
                            view! { <p class="empty">"No files to graph."</p> }.into_any()
                        }
                        Ok(g) => {
                            let positions = layout(&g.nodes, &g.edges);
                            let index_of: std::collections::HashMap<String, usize> = g
                                .nodes
                                .iter()
                                .enumerate()
                                .map(|(i, node)| (node.id.clone(), i))
                                .collect();

                            let edge_lines = g
                                .edges
                                .iter()
                                .filter_map(|e| {
                                    let a = *index_of.get(&e.from)?;
                                    let b = *index_of.get(&e.to)?;
                                    let (x1, y1) = positions[a];
                                    let (x2, y2) = positions[b];
                                    Some(view! {
                                        <line
                                            x1=x1
                                            y1=y1
                                            x2=x2
                                            y2=y2
                                            stroke="#7676764d"
                                            stroke-width="1"
                                        ></line>
                                    })
                                })
                                .collect::<Vec<_>>();

                            let node_marks = g
                                .nodes
                                .iter()
                                .enumerate()
                                .map(|(i, node)| {
                                    let (x, y) = positions[i];
                                    let color = language_color(&node.language);
                                    let label = short_label(&node.id).to_string();
                                    let title = node.id.clone();
                                    let target = node.id.clone();
                                    view! {
                                        <g
                                            style="cursor: pointer"
                                            on:click=move |_| selected.set(Some(target.clone()))
                                        >
                                            <title>{title}</title>
                                            <circle cx=x cy=y r="6" fill=color></circle>
                                            <text x=x + 8.0 y=y + 4.0 font-size="10">
                                                {label}
                                            </text>
                                        </g>
                                    }
                                })
                                .collect::<Vec<_>>();

                            view! {
                                <div>
                                    {if g.truncated {
                                        view! {
                                            <p class="empty">
                                                {format!(
                                                    "Showing the {} most-connected files.",
                                                    g.nodes.len(),
                                                )}
                                            </p>
                                        }
                                        .into_any()
                                    } else {
                                        ().into_any()
                                    }}
                                    <svg
                                        width=GRAPH_WIDTH
                                        height=GRAPH_HEIGHT
                                        viewBox=format!("0 0 {GRAPH_WIDTH} {GRAPH_HEIGHT}")
                                        style="border: 1px solid #7676764d; max-width: 100%; height: auto;"
                                    >
                                        {edge_lines}
                                        {node_marks}
                                    </svg>
                                </div>
                            }
                            .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// The Architecture section's Map sub-view (issue #352): Louvain-
/// detected communities within the dependency graph (`/api/communities`),
/// laid out as a treemap sized by code volume -- reusing the exact same
/// `squarify` layout `FilesSection`'s treemap already uses, just with
/// one tile per community instead of one tile per file.
#[component]
fn CommunitiesSection() -> impl IntoView {
    let communities = LocalResource::new(|| fetch_json::<Communities>("/api/communities"));
    const W: f64 = 900.0;
    const H: f64 = 420.0;

    view! {
        <h2>"Map"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                communities
                    .get()
                    .map(|result| match result.take() {
                        Ok(c) if c.communities.is_empty() => {
                            view! { <p class="empty">"No files to map."</p> }.into_any()
                        }
                        Ok(c) => {
                            let values: Vec<f64> =
                                c.communities.iter().map(|e| e.total_lines as f64).collect();
                            let tiles = squarify(&values, W, H);
                            let entries = c.communities.clone();
                            view! {
                                <p>
                                    {format!(
                                        "{} detected communit{} across the dependency graph. \
                                         Area is proportional to total lines of code.",
                                        entries.len(),
                                        if entries.len() == 1 { "y" } else { "ies" },
                                    )}
                                </p>
                                {c.truncated.then(|| view! {
                                    <p class="empty">
                                        {format!(
                                            "Showing the {} largest communities.",
                                            entries.len(),
                                        )}
                                    </p>
                                })}
                                <svg
                                    viewBox=format!("0 0 {W} {H}")
                                    style="width: 100%; height: auto; border: 1px solid #8884;"
                                    role="img"
                                >
                                    {tiles.into_iter().filter_map(|t| {
                                        let e = entries.get(t.index)?.clone();
                                        let fill = language_color(&e.dominant_language);
                                        let sample: Vec<&str> = e.files.iter()
                                            .take(5)
                                            .map(String::as_str)
                                            .collect();
                                        let more = if e.file_count > sample.len() {
                                            format!(", +{} more", e.file_count - sample.len())
                                        } else {
                                            String::new()
                                        };
                                        let label = format!(
                                            "{} file(s), {} line(s), mostly {} -- {}{more}",
                                            e.file_count, e.total_lines, e.dominant_language,
                                            sample.join(", "),
                                        );
                                        let short_label = format!("#{}", e.id);
                                        Some(view! {
                                            <g>
                                                <title>{label}</title>
                                                <rect
                                                    x=t.x y=t.y width=t.w height=t.h
                                                    fill=fill
                                                    stroke="#fff"
                                                    stroke-width="1"
                                                />
                                                {(t.w > 24.0 && t.h > 14.0).then(|| view! {
                                                    <text
                                                        x=t.x + 4.0 y=t.y + 14.0
                                                        font-size="10"
                                                        fill="#fff"
                                                    >
                                                        {short_label}
                                                    </text>
                                                })}
                                            </g>
                                        })
                                    }).collect::<Vec<_>>()}
                                </svg>
                            }
                            .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Confidence-tiered dead-code candidates (`/api/dead-code`), with a
/// minimum-confidence filter -- mirrors the `get_dead_code` MCP tool's
/// own `min_confidence`/`total_matching` shape.
#[component]
fn DeadCodeSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let filter = RwSignal::new(String::new());
    let dead_code = LocalResource::new(move || {
        let min_confidence = filter.get();
        async move {
            if min_confidence.is_empty() {
                fetch_json::<DeadCode>("/api/dead-code").await
            } else {
                fetch_json_with_query::<DeadCode>(
                    "/api/dead-code",
                    &[("min_confidence", &min_confidence)],
                )
                .await
            }
        }
    });

    view! {
        <h2>"Dead code"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                dead_code
                    .get()
                    .map(|result| match result.take() {
                        Ok(dc) if dc.candidates.is_empty() => {
                            view! { <p class="empty">"No dead-code candidates found."</p> }.into_any()
                        }
                        Ok(dc) => view! {
                            <label for="dead-code-confidence-filter">"Minimum confidence: "</label>
                            <select
                                id="dead-code-confidence-filter"
                                on:change=move |ev| filter.set(event_target_value(&ev))
                            >
                                <option value="">"Low (all)"</option>
                                <option value="medium">"Medium"</option>
                                <option value="high">"High"</option>
                            </select>
                            <p>{format!("{} candidate(s).", dc.total_matching)}</p>
                            <table>
                                <thead>
                                    <tr>
                                        <th>"Symbol"</th>
                                        <th>"File"</th>
                                        <th>"Line"</th>
                                        <th>"Confidence"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {dc.candidates.into_iter().map(|c| {
                                        let confidence_title = if c.risk_factors.is_empty() {
                                            "no risk factors".to_string()
                                        } else {
                                            c.risk_factors.join("; ")
                                        };
                                        view! {
                                            <tr>
                                                <td>{c.symbol}</td>
                                                <td>{file_cell(c.file, selected)}</td>
                                                <td>{c.line}</td>
                                                <td title=confidence_title>{c.confidence}</td>
                                            </tr>
                                        }
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Deterministic refactor candidates (`/api/refactor-candidates`), with a
/// kind filter -- mirrors the `get_refactor_candidates` MCP tool's own
/// `kind`/`total_matching` shape. Issue #355.
#[component]
fn RefactorCandidatesSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let filter = RwSignal::new(String::new());
    let candidates = LocalResource::new(move || {
        let kind = filter.get();
        async move {
            if kind.is_empty() {
                fetch_json::<RefactorCandidates>("/api/refactor-candidates").await
            } else {
                fetch_json_with_query::<RefactorCandidates>(
                    "/api/refactor-candidates",
                    &[("kind", &kind)],
                )
                .await
            }
        }
    });

    view! {
        <h2>"Refactoring candidates"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                candidates
                    .get()
                    .map(|result| match result.take() {
                        Ok(rc) if rc.candidates.is_empty() => {
                            view! { <p class="empty">"No refactor candidates found."</p> }.into_any()
                        }
                        Ok(rc) => view! {
                            <label for="refactor-kind-filter">"Kind: "</label>
                            <select
                                id="refactor-kind-filter"
                                on:change=move |ev| filter.set(event_target_value(&ev))
                            >
                                <option value="">"All"</option>
                                <option value="break-import-cycle">"Break import cycle"</option>
                                <option value="split-god-class">"Split god class"</option>
                                <option value="split-by-cohesion">"Split by cohesion"</option>
                                <option value="extract-duplicate">"Extract duplicate"</option>
                            </select>
                            <p>{format!("{} candidate(s).", rc.total_matching)}</p>
                            <table>
                                <thead>
                                    <tr>
                                        <th>"Title"</th>
                                        <th>"Kind"</th>
                                        <th>"Files"</th>
                                        <th>"Symbols"</th>
                                        <th>"Rationale"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {rc.candidates.into_iter().map(|c| {
                                        let last = c.files.len().saturating_sub(1);
                                        let files = c.files.into_iter()
                                            .enumerate()
                                            .map(|(i, f)| {
                                                let sep = if i < last { ", " } else { "" };
                                                view! { {file_cell(f, selected)}{sep} }.into_any()
                                            })
                                            .collect::<Vec<_>>();
                                        let symbols = c.symbols.join(", ");
                                        view! {
                                            <tr id=c.id>
                                                <td>{c.title}</td>
                                                <td>{c.kind}</td>
                                                <td>{files}</td>
                                                <td>{symbols}</td>
                                                <td>{c.rationale}</td>
                                            </tr>
                                        }
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Browsable, freshness-tracked doc index (`/api/doc-coverage`) — every
/// indexed file, whether it has a wiki page yet, and whether that page
/// still reflects the file's current source. Issue #351: replaces the
/// only-reachable-via-a-file's-own-drill-down-panel wiki access with a
/// top-level view, and answers "which docs have drifted" by comparing
/// each page's embedded content hash against the file's current one --
/// see `repowise_docs::check_freshness`'s own doc comment for why that
/// comparison is meaningful without a new `repowise docs` run.
#[component]
fn DocsSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let filter = RwSignal::new(String::new());
    let coverage = LocalResource::new(|| fetch_json::<DocCoverage>("/api/doc-coverage"));

    view! {
        <h2>"Docs"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                coverage
                    .get()
                    .map(|result| match result.take() {
                        Ok(dc) if dc.entries.is_empty() => {
                            view! { <p class="empty">"No indexed files."</p> }.into_any()
                        }
                        Ok(dc) => {
                            let status_filter = filter.get();
                            let rows: Vec<_> = dc
                                .entries
                                .iter()
                                .filter(|e| status_filter.is_empty() || e.status == status_filter)
                                .cloned()
                                .collect();
                            view! {
                                <p>
                                    {format!(
                                        "{} fresh, {} stale, {} missing.",
                                        dc.fresh, dc.stale, dc.missing,
                                    )}
                                </p>
                                <label for="doc-coverage-status-filter">"Status: "</label>
                                <select
                                    id="doc-coverage-status-filter"
                                    on:change=move |ev| filter.set(event_target_value(&ev))
                                >
                                    <option value="">"All"</option>
                                    <option value="fresh">"Fresh"</option>
                                    <option value="stale">"Stale"</option>
                                    <option value="missing">"Missing"</option>
                                </select>
                                <table>
                                    <thead>
                                        <tr>
                                            <th>"File"</th>
                                            <th>"Status"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {rows.into_iter().map(|e| view! {
                                            <tr>
                                                <td>{file_cell(e.file, selected)}</td>
                                                <td>{e.status}</td>
                                            </tr>
                                        }).collect::<Vec<_>>()}
                                    </tbody>
                                </table>
                            }
                            .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Repo-wide change coupling (`/api/coupling`) — the Architecture
/// section's Coupling sub-view (issue #352): file pairs that keep
/// changing together in the same commit, regardless of whether an
/// import edge connects them. `available: false` (no git history) shows
/// a plain message rather than an empty table, same convention as the
/// Hotspots/Ownership sections.
#[component]
fn CouplingSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let coupling = LocalResource::new(|| fetch_json::<Coupling>("/api/coupling"));

    view! {
        <h2>"Coupling"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                coupling
                    .get()
                    .map(|result| match result.take() {
                        Ok(c) if !c.available => {
                            view! { <p class="empty">"No git history found for this repo."</p> }
                                .into_any()
                        }
                        Ok(c) if c.pairs.is_empty() => {
                            view! { <p class="empty">"No coupled file pairs found."</p> }.into_any()
                        }
                        Ok(c) => view! {
                            <table>
                                <thead>
                                    <tr>
                                        <th>"File A"</th>
                                        <th>"File B"</th>
                                        <th>"Commits together"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {c.pairs.into_iter().map(|p| view! {
                                        <tr>
                                            <td>{file_cell(p.file_a, selected)}</td>
                                            <td>{file_cell(p.file_b, selected)}</td>
                                            <td>{p.count}</td>
                                        </tr>
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Days-since-epoch to `(year, month, day)`, Howard Hinnant's
/// `civil_from_days` algorithm -- a small, well-known, dependency-free
/// conversion, in keeping with this crate's own "no D3 or other JS
/// library involved" convention (this crate has no date/time
/// dependency to reach for either).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// `timestamp` (Unix seconds) as `"YYYY-MM-DD HH:MM"` UTC.
fn format_timestamp(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86400);
    let secs_of_day = timestamp.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}")
}

/// One commit's diff-shape risk score, fetched only once its row is
/// clicked (`/api/commit-risk?revspec=<hash>`) -- the same lazy,
/// load-on-selection shape `FileDetail`'s wiki/ownership/decisions
/// panels already use, and for the same reason issue #356 calls for
/// it: scoring is a real per-commit diff computation, expensive enough
/// that eagerly scoring every listed commit would multiply that cost
/// by however many are listed.
#[component]
fn CommitRiskDetail(hash: String) -> impl IntoView {
    let risk = LocalResource::new({
        let hash = hash.clone();
        move || {
            let hash = hash.clone();
            async move {
                fetch_json_with_query::<CommitRisk>("/api/commit-risk", &[("revspec", &hash)]).await
            }
        }
    });

    view! {
        <Suspense fallback=|| view! { <p>"Loading risk score..."</p> }>
            {move || {
                risk.get()
                    .map(|result| match result.take() {
                        Ok(r) => view! {
                            <ul>
                                <li>{format!("Score: {:.1} / 10", r.score)}</li>
                                <li>{format!("+{} / -{} lines", r.lines_added, r.lines_deleted)}</li>
                                <li>{format!("{} file(s), {} subsystem(s) touched", r.files_touched, r.subsystems_touched)}</li>
                                <li>{format!("Concentration: {:.2}", r.concentration)}</li>
                                <li>{format!("Author: {} ({} prior commits)", r.author, r.author_prior_commits)}</li>
                            </ul>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// The Commits view (issue #356): a bounded, recent-first list of
/// commits (`/api/commits`), each row expanding to its on-demand risk
/// score when clicked -- see `CommitRiskDetail`'s doc comment for why
/// scoring isn't eager.
#[component]
fn CommitsSection() -> impl IntoView {
    let commits = LocalResource::new(|| fetch_json::<Commits>("/api/commits"));
    let expanded = RwSignal::new(None::<String>);

    view! {
        <h2>"Commits"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                commits
                    .get()
                    .map(|result| match result.take() {
                        Ok(c) if !c.available => {
                            view! { <p class="empty">"No git history found for this repo."</p> }
                                .into_any()
                        }
                        Ok(c) if c.commits.is_empty() => {
                            view! { <p class="empty">"No commits found."</p> }.into_any()
                        }
                        Ok(c) => view! {
                            <table>
                                <thead>
                                    <tr>
                                        <th>"Commit"</th>
                                        <th>"Date"</th>
                                        <th>"Author"</th>
                                        <th>"Files"</th>
                                        <th>"Message"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {c.commits.into_iter().map(|commit| {
                                        let hash = commit.hash.clone();
                                        let row_hash = hash.clone();
                                        let detail_hash = hash.clone();
                                        let is_expanded = move || expanded.get().as_deref() == Some(hash.as_str());
                                        view! {
                                            <tr
                                                style="cursor: pointer"
                                                on:click=move |_| {
                                                    let current = expanded.get();
                                                    if current.as_deref() == Some(row_hash.as_str()) {
                                                        expanded.set(None);
                                                    } else {
                                                        expanded.set(Some(row_hash.clone()));
                                                    }
                                                }
                                            >
                                                <td>{commit.short_hash}</td>
                                                <td>{format_timestamp(commit.timestamp)}</td>
                                                <td>{commit.author}</td>
                                                <td>{commit.files_touched}</td>
                                                <td>{commit.message}</td>
                                            </tr>
                                            {move || {
                                                if is_expanded() {
                                                    view! {
                                                        <tr>
                                                            <td colspan="5">
                                                                <CommitRiskDetail hash=detail_hash.clone() />
                                                            </td>
                                                        </tr>
                                                    }
                                                    .into_any()
                                                } else {
                                                    ().into_any()
                                                }
                                            }}
                                        }
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// External (third-party) dependency registry (`/api/external-deps`) --
/// the Architecture section's Dependencies sub-view (issue #353).
/// Declared, not resolved: the version constraint exactly as written in
/// each manifest, not a lockfile-resolved version.
#[component]
fn ExternalDepsSection(selected: RwSignal<Option<String>>) -> impl IntoView {
    let filter = RwSignal::new(String::new());
    let deps = LocalResource::new(|| fetch_json::<Vec<ExternalDependency>>("/api/external-deps"));

    view! {
        <h2>"Dependencies"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                deps
                    .get()
                    .map(|result| match result.take() {
                        Ok(all) if all.is_empty() => {
                            view! { <p class="empty">"No third-party dependencies found."</p> }
                                .into_any()
                        }
                        Ok(all) => {
                            let mut ecosystems: Vec<String> =
                                all.iter().map(|d| d.ecosystem.clone()).collect();
                            ecosystems.sort();
                            ecosystems.dedup();
                            let ecosystem_filter = filter.get();
                            let rows: Vec<ExternalDependency> = all
                                .into_iter()
                                .filter(|d| ecosystem_filter.is_empty() || d.ecosystem == ecosystem_filter)
                                .collect();
                            view! {
                                <label for="deps-ecosystem-filter">"Ecosystem: "</label>
                                <select
                                    id="deps-ecosystem-filter"
                                    on:change=move |ev| filter.set(event_target_value(&ev))
                                >
                                    <option value="">"All"</option>
                                    {ecosystems.into_iter().map(|e| {
                                        view! { <option value=e.clone()>{e.clone()}</option> }
                                    }).collect::<Vec<_>>()}
                                </select>
                                <p>{format!("{} dependenc{}.", rows.len(), if rows.len() == 1 { "y" } else { "ies" })}</p>
                                <table>
                                    <thead>
                                        <tr>
                                            <th>"Name"</th>
                                            <th>"Version"</th>
                                            <th>"Kind"</th>
                                            <th>"Ecosystem"</th>
                                            <th>"Manifest"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {rows.into_iter().map(|d| {
                                            let version = d.version.unwrap_or_else(|| "-".to_string());
                                            view! {
                                                <tr>
                                                    <td>{d.name}</td>
                                                    <td>{version}</td>
                                                    <td>{d.kind}</td>
                                                    <td>{d.ecosystem}</td>
                                                    <td>{file_cell(d.file, selected)}</td>
                                                </tr>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </tbody>
                                </table>
                            }
                            .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// The live job banner (#65): a "Reindex" button plus a status line
/// polling `GET /api/reindex`. Polling starts once on mount (to pick up
/// a job already running from a previous page load) and again after
/// triggering a new one, stopping as soon as the job leaves `Running`.
#[component]
fn JobBanner() -> impl IntoView {
    let status = RwSignal::new(None::<ReindexStatus>);
    let triggering = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    let poll_until_done = move || {
        spawn_local(async move {
            loop {
                match fetch_json::<ReindexStatus>("/api/reindex").await {
                    Ok(s) => {
                        let running = matches!(s, ReindexStatus::Running);
                        status.set(Some(s));
                        if !running {
                            break;
                        }
                    }
                    Err(e) => {
                        error.set(Some(e));
                        break;
                    }
                }
                TimeoutFuture::new(500).await;
            }
        });
    };
    poll_until_done();

    let do_trigger = move || {
        if triggering.get() || matches!(status.get(), Some(ReindexStatus::Running)) {
            return;
        }
        triggering.set(true);
        error.set(None);
        spawn_local(async move {
            match post_json::<(), ReindexStatus>("/api/reindex", &()).await {
                Ok(s) => {
                    let running = matches!(s, ReindexStatus::Running);
                    status.set(Some(s));
                    triggering.set(false);
                    if running {
                        poll_until_done();
                    }
                }
                Err(e) => {
                    error.set(Some(e));
                    triggering.set(false);
                }
            }
        });
    };

    view! {
        <div class="job-banner">
            <button
                on:click=move |_| do_trigger()
                prop:disabled=move || {
                    triggering.get() || matches!(status.get(), Some(ReindexStatus::Running))
                }
            >
                "Reindex"
            </button>
            {move || {
                let text = match status.get() {
                    None => String::new(),
                    Some(ReindexStatus::Idle) => "Idle".to_string(),
                    Some(ReindexStatus::Running) => "Reindexing...".to_string(),
                    Some(ReindexStatus::Completed {
                        file_count,
                        other_file_count,
                        duration_ms,
                    }) => format!(
                        "Indexed {file_count} file(s) ({other_file_count} other) in {duration_ms}ms.",
                    ),
                    Some(ReindexStatus::Failed { error }) => format!("Reindex failed: {error}"),
                };
                view! { <span class="job-banner-status">{text}</span> }
            }}
            {move || {
                error.get().map(|e| view! { <p class="error">{format!("Error: {e}")}</p> })
            }}
        </div>
    }
}

/// A read-only Settings view over `/api/settings`: repo root, indexed
/// file counts, and whether git history / wiki pages / an LLM are
/// available -- this port has no persisted config to write to yet, so
/// there's no edit form here, just the server's current status.
#[component]
fn SettingsSection() -> impl IntoView {
    let settings = LocalResource::new(|| fetch_json::<Settings>("/api/settings"));

    view! {
        <h2>"Settings"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                settings
                    .get()
                    .map(|result| match result.take() {
                        Ok(s) => view! {
                            <ul>
                                <li>{format!("Root: {}", s.root)}</li>
                                <li>
                                    {format!(
                                        "{} indexed file(s), {} other file(s)",
                                        s.file_count, s.other_file_count,
                                    )}
                                </li>
                                <li>
                                    {format!(
                                        "Git history: {}",
                                        if s.git_available { "available" } else { "not available" },
                                    )}
                                </li>
                                <li>
                                    {format!(
                                        "Wiki pages: {}",
                                        if s.wiki_pages_available { "available" } else { "not generated" },
                                    )}
                                </li>
                                <li>
                                    {match (s.llm_configured, s.llm_model) {
                                        (true, Some(model)) => format!("LLM: configured ({model})"),
                                        (true, None) => "LLM: configured".to_string(),
                                        (false, _) => "LLM: not configured".to_string(),
                                    }}
                                </li>
                            </ul>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Issue #64's first slice: a read-only "repo cards" view over
/// `/api/workspace-repos`, listing every repo the server was started
/// with (`--workspace <path>`) and each one's indexed status. Shows a
/// plain explanatory message instead of empty cards when no workspace
/// was configured. Deliberately does NOT let you switch which repo the
/// rest of the dashboard is viewing -- that's separate future work; the
/// rest of this page keeps showing whatever repo the server's `root`
/// argument points at, same as before this section existed.
#[component]
fn WorkspaceSection() -> impl IntoView {
    let workspace = LocalResource::new(|| fetch_json::<WorkspaceRepos>("/api/workspace-repos"));

    view! {
        <h2>"Workspace"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                workspace
                    .get()
                    .map(|result| match result.take() {
                        Ok(w) if !w.available => view! {
                            <p class="empty">
                                "No workspace configured (start the server with --workspace)."
                            </p>
                        }
                        .into_any(),
                        Ok(w) => view! {
                            <div class="repo-cards">
                                {w.repos.into_iter().map(|r| view! {
                                    <div class="repo-card">
                                        <h3>{r.name}</h3>
                                        <p>{r.path}</p>
                                        <p>
                                            {if r.indexed {
                                                format!(
                                                    "{} file(s) indexed ({} other)",
                                                    r.file_count.unwrap_or(0),
                                                    r.other_file_count.unwrap_or(0),
                                                )
                                            } else {
                                                "not indexed".to_string()
                                            }}
                                        </p>
                                    </div>
                                }).collect::<Vec<_>>()}
                            </div>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// The next slice of #64 after `WorkspaceSection`: each workspace
/// repo's own most-coupled file pairs (from `GET
/// /api/workspace-co-changes`), shown side by side. NOT cross-repo
/// co-change -- separate repos have separate git histories, so files in
/// different repos can never literally co-change together -- just each
/// repo's own `repowise_git`-derived coupling rendered in one view. A
/// repo with no readable git history renders a per-card note instead of
/// an empty table.
#[component]
fn CoChangesSection() -> impl IntoView {
    let co_changes =
        LocalResource::new(|| fetch_json::<WorkspaceCoChanges>("/api/workspace-co-changes"));

    view! {
        <h2>"Workspace Co-Changes"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                co_changes
                    .get()
                    .map(|result| match result.take() {
                        Ok(w) if !w.available => view! {
                            <p class="empty">
                                "No workspace configured (start the server with --workspace)."
                            </p>
                        }
                        .into_any(),
                        Ok(w) => view! {
                            <div class="repo-cards">
                                {w.repos.into_iter().map(|r| view! {
                                    <div class="repo-card">
                                        <h3>{r.name}</h3>
                                        {if !r.available {
                                            view! {
                                                <p>"No git history found (or not a git repo)."</p>
                                            }
                                            .into_any()
                                        } else if r.pairs.is_empty() {
                                            view! {
                                                <p>"No co-change coupling found (or too little history)."</p>
                                            }
                                            .into_any()
                                        } else {
                                            view! {
                                                <ul>
                                                    {r.pairs.into_iter().map(|p| view! {
                                                        <li>
                                                            {format!("{} — {} <-> {}", p.count, p.file_a, p.file_b)}
                                                        </li>
                                                    }).collect::<Vec<_>>()}
                                                </ul>
                                            }
                                            .into_any()
                                        }}
                                    </div>
                                }).collect::<Vec<_>>()}
                            </div>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// The next slice of #64 after co-changes: real cross-repo import
/// resolution over `GET /api/workspace-architecture`, rendered as a
/// plain repo-pair table (from/to/edge count) with the individual
/// import sites listed underneath -- a table is more honest than
/// forcing this into `GraphSection`'s SVG force-layout machinery given
/// repo-level granularity is small. Covers Rust, Python, Java, Kotlin,
/// Scala, Go, C#, and PHP -- see `repowise-workspace`'s own doc comment
/// for why every other language's cross-repo imports are left
/// unresolved.
#[component]
fn SystemMapSection() -> impl IntoView {
    let architecture =
        LocalResource::new(|| fetch_json::<WorkspaceArchitecture>("/api/workspace-architecture"));

    view! {
        <h2>"System Map"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                architecture
                    .get()
                    .map(|result| match result.take() {
                        Ok(a) if !a.available => view! {
                            <p class="empty">
                                "No workspace configured (start the server with --workspace)."
                            </p>
                        }
                        .into_any(),
                        Ok(a) if a.repo_edges.is_empty() => view! {
                            <p class="empty">
                                "No cross-repo imports resolved between the configured repos."
                            </p>
                        }
                        .into_any(),
                        Ok(a) => {
                            let shown = a.edges.len();
                            let total = a.total_edges;
                            view! {
                                <table>
                                    <thead>
                                        <tr>
                                            <th>"From repo"</th>
                                            <th>"To repo"</th>
                                            <th>"Edges"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {a.repo_edges.iter().map(|e| view! {
                                            <tr>
                                                <td>{e.from_repo.clone()}</td>
                                                <td>{e.to_repo.clone()}</td>
                                                <td>{e.edge_count}</td>
                                            </tr>
                                        }).collect::<Vec<_>>()}
                                    </tbody>
                                </table>
                                <ul>
                                    {a.edges.into_iter().map(|e| view! {
                                        <li>
                                            {format!(
                                                "{} :: {} -> {} :: {} ({})",
                                                e.from_repo, e.from_file, e.to_repo, e.to_file, e.import_path,
                                            )}
                                        </li>
                                    }).collect::<Vec<_>>()}
                                </ul>
                                {(shown < total).then(|| view! {
                                    <p class="empty">
                                        {format!("Showing the first {shown} of {total} edges.")}
                                    </p>
                                })}
                            }
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// The conformance slice of #64: circular cross-repo dependencies over
/// `GET /api/workspace-conformance`, reusing exactly the edges
/// `SystemMapSection` already renders. A workspace's repo-level
/// dependency graph should form a DAG; a cycle is a concrete,
/// deterministic "pattern divergence" finding that needs no further
/// human-specified rule set to detect -- unlike contracts (still a
/// follow-up), which needs its own new detection capability entirely.
#[component]
fn ConformanceSection() -> impl IntoView {
    let conformance =
        LocalResource::new(|| fetch_json::<WorkspaceConformance>("/api/workspace-conformance"));

    view! {
        <h2>"Conformance"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                conformance
                    .get()
                    .map(|result| match result.take() {
                        Ok(c) if !c.available => view! {
                            <p class="empty">
                                "No workspace configured (start the server with --workspace)."
                            </p>
                        }
                        .into_any(),
                        Ok(c) if c.cycles.is_empty() => view! {
                            <p class="empty">"No circular cross-repo dependencies found."</p>
                        }
                        .into_any(),
                        Ok(c) => view! {
                            <ul>
                                {c.cycles.into_iter().map(|cycle| view! {
                                    <li>{cycle.join(" <-> ")}</li>
                                }).collect::<Vec<_>>()}
                            </ul>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// The last of #64's five bundled items: regex-based HTTP producer/
/// consumer route matching over `GET /api/workspace-contracts`. Fully
/// independent of the other #64 views -- no cross-repo symbol
/// resolution involved, just a fixed pattern table over raw source
/// text (see `repowise-workspace`'s `contracts` module doc comment for
/// the coarse/heuristic caveat). Renders matched producer/consumer
/// pairs, unmatched consumer calls (not necessarily a problem -- may
/// be a genuinely external API, or a producer this heuristic's pattern
/// table doesn't recognize), and -- since issue #339 -- contracts that
/// resolved on a *previous* fetch of this same endpoint and stopped
/// (leads the view when non-empty, same "warning before the numbers it
/// invalidates" convention `render_metrics` uses in the CLI): every
/// fetch here both diffs against and overwrites the persisted
/// `.repowise-workspace/contracts.json` snapshot, so polling this view
/// is itself how the baseline stays current.
#[component]
fn ContractsSection() -> impl IntoView {
    let contracts =
        LocalResource::new(|| fetch_json::<WorkspaceContracts>("/api/workspace-contracts"));

    view! {
        <h2>"Contracts"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                contracts
                    .get()
                    .map(|result| match result.take() {
                        Ok(c) if !c.available => view! {
                            <p class="empty">
                                "No workspace configured (start the server with --workspace)."
                            </p>
                        }
                        .into_any(),
                        Ok(c) => view! {
                            <div>
                                {if c.broken.is_empty() {
                                    view! { <span></span> }.into_any()
                                } else {
                                    view! {
                                        <div class="warning">
                                            <h3>{format!("Broken ({})", c.broken.len())}</h3>
                                            <ul>
                                                {c.broken.into_iter().map(|b| view! {
                                                    <li>
                                                        {format!(
                                                            "{} ({} :: {}) used to resolve to {} -- {}",
                                                            b.path, b.consumer_repo, b.consumer_file,
                                                            b.previous_producer_repo,
                                                            b.reason.as_deref().unwrap_or(
                                                                "the consumer call site itself is gone",
                                                            ),
                                                        )}
                                                    </li>
                                                }).collect::<Vec<_>>()}
                                            </ul>
                                        </div>
                                    }
                                    .into_any()
                                }}
                                <h3>"Matched"</h3>
                                {if c.matches.is_empty() {
                                    view! { <p class="empty">"No cross-repo API contracts matched."</p> }.into_any()
                                } else {
                                    view! {
                                        <ul>
                                            {c.matches.into_iter().map(|m| view! {
                                                <li>
                                                    {format!(
                                                        "{}: {} ({}) <- {} ({})",
                                                        m.path, m.producer_repo, m.producer_file,
                                                        m.consumer_repo, m.consumer_file,
                                                    )}
                                                </li>
                                            }).collect::<Vec<_>>()}
                                        </ul>
                                    }
                                    .into_any()
                                }}
                                <h3>"Unmatched consumer calls"</h3>
                                {if c.unmatched_consumers.is_empty() {
                                    view! { <p class="empty">"None found."</p> }.into_any()
                                } else {
                                    view! {
                                        <ul>
                                            {c.unmatched_consumers.into_iter().map(|u| view! {
                                                <li>{format!("{} ({} :: {})", u.path, u.repo, u.file)}</li>
                                            }).collect::<Vec<_>>()}
                                        </ul>
                                    }
                                    .into_any()
                                }}
                            </div>
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// A cost-tracking view over `/api/usage` (#65's last remaining bundled
/// feature): running token counts tallied across every `/api/chat`
/// call this server process has handled, not a persisted history and
/// not a dollar figure -- see `repowise_llm::Usage`'s own doc comment
/// for why there's no pricing conversion. Polls every 3s (rather than a
/// one-shot fetch like most other sections) so it keeps reflecting
/// `ChatSection`'s activity elsewhere on the page without the two
/// components needing to share any state directly.
#[component]
fn UsageSection() -> impl IntoView {
    let usage = RwSignal::new(None::<Usage>);
    let error = RwSignal::new(None::<String>);

    spawn_local(async move {
        loop {
            match fetch_json::<Usage>("/api/usage").await {
                Ok(u) => {
                    usage.set(Some(u));
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
            TimeoutFuture::new(3000).await;
        }
    });

    view! {
        <h2>"Usage"</h2>
        {move || match usage.get() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(u) => view! {
                <ul>
                    <li>{format!("{} chat call(s)", u.chat_call_count)}</li>
                    <li>{format!("{} prompt token(s)", u.prompt_tokens)}</li>
                    <li>{format!("{} completion token(s)", u.completion_tokens)}</li>
                    <li>{format!("{} total token(s)", u.total_tokens)}</li>
                </ul>
                <p class="empty">
                    "Token counts for this server process only (resets on restart), \
                     not a dollar cost -- see the README for why."
                </p>
            }
            .into_any(),
        }}
        {move || error.get().map(|e| view! { <p class="error">{format!("Error: {e}")}</p> })}
    }
}

/// A chat interface over `/api/chat`. Renders a plain explanatory
/// message instead of a chat box when the server reports the LLM
/// feature isn't configured, rather than a confusing empty/broken UI.
#[component]
fn ChatSection() -> impl IntoView {
    let history: RwSignal<Vec<ChatTurn>> = RwSignal::new(Vec::new());
    let draft = RwSignal::new(String::new());
    let sending = RwSignal::new(false);
    let unavailable = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    let do_send = move || {
        let message = draft.get();
        if message.trim().is_empty() || sending.get() {
            return;
        }
        draft.set(String::new());
        error.set(None);
        history.update(|h| {
            h.push(ChatTurn {
                role: "user".to_string(),
                content: message,
            })
        });
        sending.set(true);
        let request_history = history.get();
        spawn_local(async move {
            let result = post_json::<ChatRequest, ChatResponse>(
                "/api/chat",
                &ChatRequest {
                    history: request_history,
                },
            )
            .await;
            match result {
                Ok(res) if !res.available => unavailable.set(true),
                Ok(res) => {
                    if let Some(reply) = res.reply {
                        history.update(|h| {
                            h.push(ChatTurn {
                                role: "assistant".to_string(),
                                content: reply,
                            })
                        });
                    }
                }
                Err(e) => error.set(Some(e)),
            }
            sending.set(false);
        });
    };

    view! {
        <h2>"Chat"</h2>
        {move || {
            if unavailable.get() {
                view! {
                    <p class="empty">
                        "Chat requires REPOWISE_LLM_BASE_URL (an OpenAI-compatible endpoint, \
                         e.g. rusty_provider) to be set on the server."
                    </p>
                }
                .into_any()
            } else {
                view! {
                    <div class="chat">
                        <ul class="chat-history">
                            {move || {
                                history
                                    .get()
                                    .into_iter()
                                    .map(|turn| view! {
                                        <li class=format!("chat-turn chat-turn-{}", turn.role)>
                                            <strong>{format!("{}: ", turn.role)}</strong>
                                            {turn.content}
                                        </li>
                                    })
                                    .collect::<Vec<_>>()
                            }}
                        </ul>
                        {move || {
                            error
                                .get()
                                .map(|e| view! { <p class="error">{format!("Error: {e}")}</p> })
                        }}
                        <input
                            type="text"
                            placeholder="Ask about this codebase..."
                            prop:value=move || draft.get()
                            prop:disabled=move || sending.get()
                            on:input=move |ev| draft.set(event_target_value(&ev))
                            on:keydown=move |ev| {
                                if ev.key() == "Enter" {
                                    do_send();
                                }
                            }
                        />
                        <button on:click=move |_| do_send() prop:disabled=move || sending.get()>
                            {move || if sending.get() { "Sending..." } else { "Send" }}
                        </button>
                    </div>
                }
                .into_any()
            }
        }}
    }
}

/// Number of slides in Present Mode -- keep in lockstep with
/// [`slide_title`] and the `match` in [`PresentMode`]'s view.
const PRESENT_SLIDE_COUNT: usize = 5;

fn slide_title(step: usize) -> &'static str {
    match step {
        0 => "Overview",
        1 => "Code health",
        2 => "Hotspots",
        3 => "Architectural decisions",
        _ => "Dependency graph",
    }
}

/// Reads `#present/<n>` from the current URL on load, so a shared or
/// bookmarked link opens directly into that slide -- the "shareable
/// state via URL" issue #65 asks for.
fn parse_present_hash() -> Option<usize> {
    let hash = web_sys::window()?.location().hash().ok()?;
    hash.strip_prefix("#present/")?.parse::<usize>().ok()
}

/// Updates the URL hash to reflect the current slide (or clears it on
/// exit) via `replaceState`, not a hash assignment, so stepping through
/// slides doesn't spam the browser's back-button history.
fn set_present_hash(step: Option<usize>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(history) = window.history() else {
        return;
    };
    let url = match step {
        Some(n) => format!("#present/{n}"),
        // A single space, not "", since some browsers ignore an empty
        // `url` argument to replaceState rather than clearing the hash.
        None => " ".to_string(),
    };
    let _ = history.replace_state_with_url(&leptos::wasm_bindgen::JsValue::NULL, "", Some(&url));
}

/// A full-screen, keyboard-driven step-through of the dashboard's core
/// narrative sections -- issue #65's Present Mode. Frontend-only: every
/// slide reuses an existing section component and the same `/api/*`
/// data it already fetches, no new server endpoint. Renders nothing
/// when `step` is `None` (present mode isn't active).
#[component]
fn PresentMode(step: RwSignal<Option<usize>>, selected: RwSignal<Option<String>>) -> impl IntoView {
    window_event_listener_untyped("keydown", move |ev| {
        let Some(current) = step.get_untracked() else {
            return;
        };
        let Some(kb) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        match kb.key().as_str() {
            "ArrowRight" | " " => {
                let next = (current + 1).min(PRESENT_SLIDE_COUNT - 1);
                step.set(Some(next));
                set_present_hash(Some(next));
            }
            "ArrowLeft" => {
                let prev = current.saturating_sub(1);
                step.set(Some(prev));
                set_present_hash(Some(prev));
            }
            "Escape" => {
                step.set(None);
                set_present_hash(None);
            }
            _ => {}
        }
    });

    view! {
        {move || {
            step.get().map(|current| {
                view! {
                    <div style="position: fixed; inset: 0; z-index: 1000; background: Canvas; \
                                 color: CanvasText; overflow: auto; padding: 2rem;">
                        <div style="display: flex; justify-content: space-between; align-items: center;">
                            <strong>
                                {format!(
                                    "{} ({} / {})",
                                    slide_title(current),
                                    current + 1,
                                    PRESENT_SLIDE_COUNT,
                                )}
                            </strong>
                            <button on:click=move |_| {
                                step.set(None);
                                set_present_hash(None);
                            }>"Exit (Esc)"</button>
                        </div>
                        <div style="margin-top: 1.5rem;">
                            {match current {
                                0 => view! { <OverviewSection selected=selected /> }.into_any(),
                                1 => view! { <HealthSection selected=selected /> }.into_any(),
                                2 => view! { <HotspotsSection selected=selected /> }.into_any(),
                                3 => view! { <DecisionsSection /> }.into_any(),
                                _ => view! { <GraphSection selected=selected /> }.into_any(),
                            }}
                        </div>
                        <div style="margin-top: 1.5rem; display: flex; gap: 0.5rem;">
                            <button
                                on:click=move |_| {
                                    let prev = current.saturating_sub(1);
                                    step.set(Some(prev));
                                    set_present_hash(Some(prev));
                                }
                                prop:disabled=current == 0
                            >
                                "< Prev"
                            </button>
                            <button
                                on:click=move |_| {
                                    let next = (current + 1).min(PRESENT_SLIDE_COUNT - 1);
                                    step.set(Some(next));
                                    set_present_hash(Some(next));
                                }
                                prop:disabled=current == PRESENT_SLIDE_COUNT - 1
                            >
                                "Next >"
                            </button>
                        </div>
                    </div>
                }
            })
        }}
    }
}

#[component]
fn App() -> impl IntoView {
    let wiki_pages: WikiPages = LocalResource::new(|| fetch_json::<Vec<String>>("/api/wiki-pages"));
    let selected = RwSignal::new(None::<String>);
    let present_step = RwSignal::new(parse_present_hash());

    // Route comes from the URL hash, so a reload restores the current
    // view instead of resetting to Overview.
    let current = RwSignal::new(parse_hash_full(&location_hash()).0);
    let detail_id = RwSignal::new(parse_hash_full(&location_hash()).2);
    if let Some(file) = parse_hash_full(&location_hash()).1 {
        selected.set(Some(file));
    }

    // Back/forward and manual edits both fire `hashchange`.
    window_event_listener_untyped("hashchange", move |_| {
        let (route, file) = parse_hash(&location_hash());
        // A present-mode hash must not move the underlying view.
        if !location_hash()
            .trim_start_matches('#')
            .starts_with("present/")
        {
            current.set(route);
            selected.set(file);
        }
    });

    // Keep the hash in step when a section sets the selection (e.g.
    // clicking a treemap tile), so that state survives a reload too.
    Effect::new(move |_| {
        let file = selected.get();
        let route = current.get();
        if location_hash()
            .trim_start_matches('#')
            .starts_with("present/")
        {
            return;
        }
        // Don't rewrite the hash while a detail view is addressed --
        // that would drop the `id` and bounce back to the index.
        if detail_id.get().is_some() {
            return;
        }
        let want = format_hash(route, file.as_deref());
        if location_hash() != want {
            replace_hash(&want);
        }
    });

    view! {
        <h1>"repowise dashboard"</h1>
        <p class="subtitle">"live server"</p>
        <JobBanner />
        <button on:click=move |_| {
            present_step.set(Some(0));
            set_present_hash(Some(0));
        }>"Present"</button>
        <SearchBox selected=selected />
        <PresentMode step=present_step selected=selected />
        <nav class="view-nav">
            {ROUTES.iter().map(|(r, slug, label)| {
                let route = *r;
                let slug = *slug;
                view! {
                    <a
                        href=move || format_hash(route, selected.get().as_deref())
                        class:active=move || current.get() == route
                        data-slug=slug
                    >{*label}</a>
                }
            }).collect::<Vec<_>>()}
        </nav>
        <FileDetailPanel wiki_pages=wiki_pages selected=selected />
        {move || match (current.get(), detail_id.get()) {
            (Route::Symbols, Some(id)) => {
                view! { <SymbolDetailSection id=id selected=selected /> }.into_any()
            }
            (Route::Decisions, Some(id)) => {
                view! { <DecisionDetailSection id=id selected=selected /> }.into_any()
            }
            (route, _) => match route {
            Route::Overview => view! { <OverviewSection selected=selected /> }.into_any(),
            Route::Health => view! { <HealthSection selected=selected /> }.into_any(),
            Route::Coverage => view! { <CoverageSection selected=selected /> }.into_any(),
            Route::Hotspots => view! { <HotspotsSection selected=selected /> }.into_any(),
            Route::Contributors => view! { <ContributorsSection /> }.into_any(),
            Route::Files => view! { <FilesSection selected=selected /> }.into_any(),
            Route::Stats => view! { <StatsSection /> }.into_any(),
            Route::Decisions => view! { <DecisionsSection /> }.into_any(),
            Route::Symbols => view! { <SymbolsSection selected=selected /> }.into_any(),
            Route::Graph => view! { <GraphSection selected=selected /> }.into_any(),
            Route::DeadCode => view! { <DeadCodeSection selected=selected /> }.into_any(),
            Route::RefactorCandidates => {
                view! { <RefactorCandidatesSection selected=selected /> }.into_any()
            }
            Route::Docs => view! { <DocsSection selected=selected /> }.into_any(),
            Route::Coupling => view! { <CouplingSection selected=selected /> }.into_any(),
            Route::ExternalDeps => view! { <ExternalDepsSection selected=selected /> }.into_any(),
            Route::Commits => view! { <CommitsSection /> }.into_any(),
            Route::Communities => view! { <CommunitiesSection /> }.into_any(),
            Route::Chat => view! { <ChatSection /> }.into_any(),
            Route::Usage => view! { <UsageSection /> }.into_any(),
            Route::Settings => view! { <SettingsSection /> }.into_any(),
            Route::Workspace => view! { <WorkspaceSection /> }.into_any(),
            Route::CoChanges => view! { <CoChangesSection /> }.into_any(),
            Route::SystemMap => view! { <SystemMapSection /> }.into_any(),
            Route::Conformance => view! { <ConformanceSection /> }.into_any(),
            Route::Contracts => view! { <ContractsSection /> }.into_any(),
            Route::NotFound => view! {
                <h2>"Not found"</h2>
                <p class="empty">
                    "No view matches that address. It may be a stale link -- pick a view above."
                </p>
            }
            .into_any(),
            },
        }}
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_timestamp_renders_the_unix_epoch() {
        assert_eq!(format_timestamp(0), "1970-01-01 00:00");
    }

    #[test]
    fn format_timestamp_renders_a_known_recent_date() {
        // 2024-01-15 12:30:00 UTC.
        assert_eq!(format_timestamp(1_705_321_800), "2024-01-15 12:30");
    }

    /// Values must reach `squarify` sorted descending (the server sorts
    /// them); these fixtures mirror that.
    fn areas(tiles: &[Tile]) -> Vec<f64> {
        tiles.iter().map(|t| t.w * t.h).collect()
    }

    #[test]
    fn tiles_fill_the_whole_rectangle() {
        // If the areas don't sum to the canvas, the treemap is lying
        // about proportion -- the one thing it exists to convey.
        let values = vec![50.0, 30.0, 12.0, 5.0, 3.0];
        let tiles = squarify(&values, 400.0, 300.0);
        assert_eq!(tiles.len(), values.len());
        let covered: f64 = areas(&tiles).iter().sum();
        assert!(
            (covered - 400.0 * 300.0).abs() < 1.0,
            "covered {covered} of {}",
            400.0 * 300.0
        );
    }

    #[test]
    fn tile_area_is_proportional_to_its_value() {
        let values = vec![60.0, 30.0, 10.0];
        let tiles = squarify(&values, 200.0, 100.0);
        let total_area = 200.0 * 100.0;
        for t in &tiles {
            let expected = values[t.index] / 100.0 * total_area;
            let actual = t.w * t.h;
            assert!(
                (actual - expected).abs() < 1.0,
                "index {}: expected {expected}, got {actual}",
                t.index
            );
        }
    }

    #[test]
    fn every_tile_stays_inside_the_canvas() {
        let values = vec![40.0, 25.0, 15.0, 10.0, 6.0, 4.0];
        for t in squarify(&values, 300.0, 200.0) {
            assert!(t.x >= -0.001 && t.y >= -0.001, "{t:?}");
            assert!(t.x + t.w <= 300.001, "{t:?}");
            assert!(t.y + t.h <= 200.001, "{t:?}");
            assert!(t.w >= 0.0 && t.h >= 0.0, "{t:?}");
        }
    }

    #[test]
    fn layout_is_deterministic() {
        // The view must not reshuffle between loads.
        let values = vec![30.0, 20.0, 15.0, 10.0, 5.0];
        assert_eq!(
            squarify(&values, 500.0, 400.0),
            squarify(&values, 500.0, 400.0)
        );
    }

    #[test]
    fn degenerate_inputs_produce_no_tiles_rather_than_panicking() {
        assert!(squarify(&[], 100.0, 100.0).is_empty());
        assert!(squarify(&[0.0, 0.0], 100.0, 100.0).is_empty());
        assert!(squarify(&[10.0], 0.0, 100.0).is_empty());
        assert!(squarify(&[10.0], 100.0, 0.0).is_empty());
    }

    #[test]
    fn a_single_file_takes_the_whole_canvas() {
        let tiles = squarify(&[42.0], 120.0, 80.0);
        assert_eq!(tiles.len(), 1);
        assert!((tiles[0].w * tiles[0].h - 120.0 * 80.0).abs() < 0.001);
    }

    #[test]
    fn unscored_is_its_own_band_and_never_reads_as_healthy() {
        // A file with no score is not a good file. Folding it into
        // "good" would color unknown risk green.
        let (label, unscored_fill) = health_band(None);
        assert_eq!(label, "unscored");
        let (good_label, good_fill) = health_band(Some(9.0));
        assert_eq!(good_label, "good");
        assert_ne!(unscored_fill, good_fill);
    }

    #[test]
    fn health_bands_split_at_the_documented_boundaries() {
        assert_eq!(health_band(Some(8.0)).0, "good");
        assert_eq!(health_band(Some(7.99)).0, "fair");
        assert_eq!(health_band(Some(5.0)).0, "fair");
        assert_eq!(health_band(Some(4.99)).0, "poor");
        assert_eq!(health_band(Some(0.0)).0, "poor");
    }

    #[test]
    fn an_empty_or_blank_query_is_never_sent() {
        // `/api/search` returns nothing for an empty needle, so sending
        // one is pure waste -- and its empty result would render as "no
        // matches" rather than "you haven't typed anything yet".
        assert!(!should_query(""));
        assert!(!should_query("   "));
        assert!(!should_query("\t\n"));
    }

    #[test]
    fn a_real_query_is_sent_even_with_surrounding_whitespace() {
        assert!(should_query("parser"));
        assert!(should_query("  parser  "));
    }

    #[test]
    fn every_route_in_the_table_round_trips() {
        // The table is the single source of truth for nav, parser and
        // formatter; a slug that doesn't round-trip is a dead link.
        for (route, slug, label) in ROUTES {
            assert!(!label.is_empty(), "{slug} has no label");
            let (parsed, sel) = parse_hash(&format_hash(*route, None));
            assert_eq!(parsed, *route, "slug {slug}");
            assert_eq!(sel, None);
        }
    }

    #[test]
    fn an_empty_or_missing_hash_lands_on_overview() {
        assert_eq!(parse_hash("").0, Route::Overview);
        assert_eq!(parse_hash("#").0, Route::Overview);
        assert_eq!(parse_hash("#/").0, Route::Overview);
    }

    #[test]
    fn an_unknown_route_is_not_found_rather_than_a_silent_redirect() {
        // A stale bookmark should say so, not quietly show Overview as
        // though nothing were wrong.
        assert_eq!(parse_hash("#/nope").0, Route::NotFound);
        assert_eq!(parse_hash("#/health/extra").0, Route::NotFound);
    }

    #[test]
    fn present_mode_does_not_clobber_the_underlying_view() {
        // `#present/<n>` is an overlay, not a view. Parsing it as a
        // route would make exiting present mode land somewhere random.
        assert_eq!(parse_hash("#present/0").0, Route::Overview);
        assert_eq!(parse_hash("#present/3").0, Route::Overview);
    }

    #[test]
    fn the_selected_file_survives_a_round_trip() {
        let h = format_hash(Route::Health, Some("crates/repowise-cli/src/main.rs"));
        let (route, sel) = parse_hash(&h);
        assert_eq!(route, Route::Health);
        assert_eq!(sel.as_deref(), Some("crates/repowise-cli/src/main.rs"));
    }

    #[test]
    fn a_path_with_hash_query_or_space_characters_round_trips() {
        // These would otherwise split the hash and lose the tail.
        for path in [
            "a b/c.rs",
            "weird#name.rs",
            "q?.rs",
            "a&b.rs",
            "100%.rs",
            "a+b.rs",
        ] {
            let (_, sel) = parse_hash(&format_hash(Route::Files, Some(path)));
            assert_eq!(sel.as_deref(), Some(path), "path {path}");
        }
    }

    #[test]
    fn an_empty_file_param_is_treated_as_no_selection() {
        assert_eq!(parse_hash("#/files?file=").1, None);
        assert_eq!(format_hash(Route::Files, Some("")), "#/files");
    }

    #[test]
    fn a_detail_id_round_trips_through_the_hash() {
        let h = format_detail_hash(Route::Symbols, "crates/a/src/lib.rs@42");
        let (route, _, id) = parse_hash_full(&h);
        assert_eq!(route, Route::Symbols);
        assert_eq!(id.as_deref(), Some("crates/a/src/lib.rs@42"));
    }

    #[test]
    fn a_decision_id_with_awkward_characters_survives() {
        for id in ["ADR-0001", "commit:abc#123", "a b&c", "x?y"] {
            let (_, _, got) = parse_hash_full(&format_detail_hash(Route::Decisions, id));
            assert_eq!(got.as_deref(), Some(id), "id {id}");
        }
    }

    #[test]
    fn an_index_route_carries_no_detail_id() {
        // Otherwise every index view would try to render a detail page.
        let (_, _, id) = parse_hash_full("#/symbols");
        assert_eq!(id, None);
        let (_, _, id) = parse_hash_full("#/decisions?file=a.rs");
        assert_eq!(id, None);
    }
}
