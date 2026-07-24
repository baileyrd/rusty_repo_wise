//! Phase 3 of the #59/#65 live-dashboard pivot adds a dependency-graph
//! view: an SVG rendering of `/api/graph`'s file-level import graph,
//! laid out client-side with a small force-directed simulation (no D3
//! or other JS graph library -- keeping the whole frontend buildable
//! with just `cargo`/`trunk`). Phase 2 added wiki-page drill-down links
//! (file paths become clickable when `repowise-docs` has already
//! generated a wiki page for them, opening its raw markdown inline) and
//! an instant, Ctrl/Cmd+K search box over files and symbols. Still not
//! full parity with real repowise's dashboard -- the chat/LLM views are
//! a later phase, not built here.

use leptos::html;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use leptos::web_sys;
use serde::Deserialize;

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

type WikiPages = LocalResource<Result<Vec<String>, String>>;

fn wiki_pages_snapshot(wiki_pages: WikiPages) -> Vec<String> {
    wiki_pages
        .get()
        .and_then(|r| r.take().ok())
        .unwrap_or_default()
}

/// A file-path table cell: a link that opens the file's wiki page
/// inline (via `selected`) when `wiki_pages` already has one on disk,
/// plain text otherwise -- never a broken link. Mirrors the static
/// dashboard's own "check disk, link if present" convention.
fn file_cell(
    path: String,
    wiki_pages: &[String],
    selected: RwSignal<Option<String>>,
) -> impl IntoView {
    if wiki_pages.contains(&path) {
        let target = path.clone();
        view! {
            <a href="#" on:click=move |ev| {
                ev.prevent_default();
                selected.set(Some(target.clone()));
            }>{path}</a>
        }
        .into_any()
    } else {
        view! { <span>{path}</span> }.into_any()
    }
}

/// Every section below follows the same shape: fetch its own resource,
/// show a loading placeholder via `Suspense`, then render either the
/// data or an error -- mirroring the static dashboard's one-section-at-
/// a-time layout, but each section now loads independently instead of
/// blocking on a single whole-page render.
#[component]
fn OverviewSection(wiki_pages: WikiPages, selected: RwSignal<Option<String>>) -> impl IntoView {
    let overview = LocalResource::new(|| fetch_json::<Overview>("/api/overview"));

    view! {
        <h2>"Overview"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                overview
                    .get()
                    .map(|result| match result.take() {
                        Ok(o) => {
                            let pages = wiki_pages_snapshot(wiki_pages);
                            view! {
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
                                                        <td>{file_cell(file, &pages, selected)}</td>
                                                        <td>{count}</td>
                                                    </tr>
                                                }).collect::<Vec<_>>()}
                                            </tbody>
                                        </table>
                                    }
                                    .into_any()
                                }}
                            }
                            .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

#[component]
fn HealthSection(wiki_pages: WikiPages, selected: RwSignal<Option<String>>) -> impl IntoView {
    let health = LocalResource::new(|| fetch_json::<Health>("/api/health"));

    view! {
        <h2>"Code health"</h2>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || {
                health
                    .get()
                    .map(|result| match result.take() {
                        Ok(h) => {
                            let pages = wiki_pages_snapshot(wiki_pages);
                            view! {
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
                                                        <td>{file_cell(f.file, &pages, selected)}</td>
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
                            .into_any()
                        }
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

#[component]
fn HotspotsSection(wiki_pages: WikiPages, selected: RwSignal<Option<String>>) -> impl IntoView {
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
                        Ok(h) => {
                            let pages = wiki_pages_snapshot(wiki_pages);
                            view! {
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
                                                <td>{file_cell(hs.file, &pages, selected)}</td>
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
                            .into_any()
                        }
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
fn SymbolsSection(wiki_pages: WikiPages, selected: RwSignal<Option<String>>) -> impl IntoView {
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
                            let pages = wiki_pages_snapshot(wiki_pages);
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
                                                        <td>{file_cell(s.file.clone(), &pages, selected)}</td>
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

/// Fetches and renders one file's wiki-page markdown inline, as raw
/// text -- there's no markdown-rendering dependency in this crate yet,
/// and pulling one in just for this would be more than Phase 2 needs.
#[component]
fn WikiContent(path: String, selected: RwSignal<Option<String>>) -> impl IntoView {
    let content = LocalResource::new({
        let path = path.clone();
        move || {
            let path = path.clone();
            async move { fetch_json_with_query::<WikiPage>("/api/wiki", &[("path", &path)]).await }
        }
    });
    let title = path.clone();

    view! {
        <div class="wiki-viewer">
            <div class="wiki-viewer-header">
                <strong>{title}</strong>
                <button on:click=move |_| selected.set(None)>"Close"</button>
            </div>
            <Suspense fallback=|| view! { <p>"Loading..."</p> }>
                {move || {
                    content
                        .get()
                        .map(|result| match result.take() {
                            Ok(w) => view! { <pre>{w.content}</pre> }.into_any(),
                            Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                        })
                }}
            </Suspense>
        </div>
    }
}

#[component]
fn WikiViewer(selected: RwSignal<Option<String>>) -> impl IntoView {
    view! {
        {move || {
            selected.get().map(|path| view! { <WikiContent path=path selected=selected /> })
        }}
    }
}

/// A Ctrl/Cmd+K instant search box over files and symbols. Results are
/// live-fetched from `/api/search` as you type; clicking a file result
/// opens its wiki page the same way a drill-down link does.
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
fn GraphSection(wiki_pages: WikiPages, selected: RwSignal<Option<String>>) -> impl IntoView {
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
                            let pages = wiki_pages_snapshot(wiki_pages);
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
                                    let has_wiki = pages.contains(&node.id);
                                    let target = node.id.clone();
                                    let cursor = if has_wiki { "pointer" } else { "default" };
                                    view! {
                                        <g
                                            style=format!("cursor: {cursor}")
                                            on:click=move |_| {
                                                if has_wiki {
                                                    selected.set(Some(target.clone()));
                                                }
                                            }
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

#[component]
fn App() -> impl IntoView {
    let wiki_pages: WikiPages = LocalResource::new(|| fetch_json::<Vec<String>>("/api/wiki-pages"));
    let selected = RwSignal::new(None::<String>);

    view! {
        <h1>"repowise dashboard"</h1>
        <p class="subtitle">"live server"</p>
        <SearchBox selected=selected />
        <WikiViewer selected=selected />
        <OverviewSection wiki_pages=wiki_pages selected=selected />
        <HealthSection wiki_pages=wiki_pages selected=selected />
        <HotspotsSection wiki_pages=wiki_pages selected=selected />
        <DecisionsSection />
        <SymbolsSection wiki_pages=wiki_pages selected=selected />
        <GraphSection wiki_pages=wiki_pages selected=selected />
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
