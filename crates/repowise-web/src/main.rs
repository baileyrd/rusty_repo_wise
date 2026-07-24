//! Phase 2 of the #59/#65 live-dashboard pivot: adds wiki-page
//! drill-down links (file paths become clickable when
//! `repowise-docs` has already generated a wiki page for them, opening
//! its raw markdown inline) and an instant, Ctrl/Cmd+K search box over
//! files and symbols, on top of Phase 1's ported views. Still not full
//! parity with real repowise's dashboard -- the dependency-graph view
//! and the chat/LLM views are later phases, not built here.

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
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
