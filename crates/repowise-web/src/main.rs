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
//! `SystemMapSection` is the next #64 slice: real cross-repo Rust `use`
//! resolution over `GET /api/workspace-architecture`, rendered as a
//! plain repo-pair table with the individual import sites listed
//! underneath. Rust-only.
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

/// Mirrors `repowise-server`'s `WorkspaceContractsDto` wire shape.
#[derive(Deserialize, Clone, Debug)]
struct WorkspaceContracts {
    available: bool,
    matches: Vec<ContractMatch>,
    unmatched_consumers: Vec<UnmatchedConsumer>,
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
    let decisions = LocalResource::new(|| fetch_json::<Vec<Decision>>("/api/decisions"));

    view! {
        <h2>"Architectural decisions"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                decisions
                    .get()
                    .map(|result| match result.take() {
                        Ok(ds) if ds.is_empty() => view! {
                            <p class="empty">
                                "No decisions found (docs/adr/*.md or decision-like commits)."
                            </p>
                        }
                        .into_any(),
                        Ok(ds) => view! {
                            <table>
                                <thead>
                                    <tr>
                                        <th>"ID"</th>
                                        <th>"Title"</th>
                                        <th>"Status"</th>
                                        <th>"Linked files"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {ds.into_iter().map(|d| {
                                        let status = match d.superseded_by {
                                            Some(target) => format!("superseded by {target}"),
                                            None => d.status.unwrap_or_else(|| "commit".to_string()),
                                        };
                                        view! {
                                            <tr>
                                                <td>{d.id}</td>
                                                <td>{d.title}</td>
                                                <td>{status}</td>
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
                                                        <td>{s.name.clone()}</td>
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
            async move {
                fetch_json_with_query::<Vec<Decision>>("/api/decisions", &[("file", &path)]).await
            }
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
                            Ok(ds) if ds.is_empty() => {
                                view! { <p class="empty">"No decisions linked to this file."</p> }
                                    .into_any()
                            }
                            Ok(ds) => view! {
                                <ul>
                                    {ds.into_iter().map(|d| view! {
                                        <li><strong>{d.id}</strong>": "{d.title}</li>
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
            if q.trim().is_empty() {
                Ok(SearchResults {
                    files: Vec::new(),
                    symbols: Vec::new(),
                })
            } else {
                fetch_json_with_query::<SearchResults>("/api/search", &[("q", &q)]).await
            }
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
                    if query.get().trim().is_empty() {
                        return None;
                    }
                    results.get().map(|result| match result.take() {
                        Ok(res) if res.files.is_empty() && res.symbols.is_empty() => {
                            view! { <p class="empty">"No matches."</p> }.into_any()
                        }
                        Ok(res) => view! {
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
                                {res.symbols.into_iter().map(|s| view! {
                                    <li class="mono">
                                        {format!("{} ({}) — {}:{}", s.name, s.kind, s.file, s.start_line)}
                                    </li>
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
    let graph = LocalResource::new(|| fetch_json::<Graph>("/api/graph"));

    view! {
        <h2>"Dependency graph"</h2>
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

/// The next slice of #64 after co-changes: real cross-repo Rust `use`
/// resolution over `GET /api/workspace-architecture`, rendered as a
/// plain repo-pair table (from/to/edge count) with the individual
/// import sites listed underneath -- a table is more honest than
/// forcing this into `GraphSection`'s SVG force-layout machinery given
/// repo-level granularity is small. Rust-only -- see
/// `repowise-workspace`'s own doc comment for why every other
/// language's cross-repo imports are left unresolved.
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
                                "No cross-repo Rust imports resolved between the configured repos."
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
/// pairs and unmatched consumer calls (not necessarily a problem -- may
/// be a genuinely external API, or a producer this heuristic's pattern
/// table doesn't recognize) as two separate lists.
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
        <FileDetailPanel wiki_pages=wiki_pages selected=selected />
        <OverviewSection selected=selected />
        <HealthSection selected=selected />
        <CoverageSection selected=selected />
        <HotspotsSection selected=selected />
        <ContributorsSection />
        <DecisionsSection />
        <SymbolsSection selected=selected />
        <GraphSection selected=selected />
        <DeadCodeSection selected=selected />
        <ChatSection />
        <UsageSection />
        <SettingsSection />
        <WorkspaceSection />
        <CoChangesSection />
        <SystemMapSection />
        <ConformanceSection />
        <ContractsSection />
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
