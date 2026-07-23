//! Phase 1 of the #59/#65 live-dashboard pivot: a Leptos CSR app that
//! ports every view the static `repowise dashboard` page already had
//! (health, hotspots, decisions, symbols) onto `repowise-server`'s JSON
//! API, on top of Phase 0's overview section. Still not full parity
//! with real repowise's dashboard -- instant search, the
//! dependency-graph view, and the chat/LLM views are later phases, not
//! built here.

use leptos::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
struct Overview {
    file_count: usize,
    other_file_count: usize,
    total_lines: usize,
    import_edges: usize,
    call_edges: usize,
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

async fn fetch_json<T>(path: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let response = gloo_net::http::Request::get(path)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.ok() {
        return Err(format!("server returned {}", response.status()));
    }
    response.json::<T>().await.map_err(|e| e.to_string())
}

/// Every section below follows the same shape: fetch its own resource,
/// show a loading placeholder via `Suspense`, then render either the
/// data or an error -- mirroring the static dashboard's one-section-at-
/// a-time layout, but each section now loads independently instead of
/// blocking on a single whole-page render.
#[component]
fn OverviewSection() -> impl IntoView {
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
                        }
                        .into_any(),
                        Err(e) => view! { <p class="error">{format!("Error: {e}")}</p> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

#[component]
fn HealthSection() -> impl IntoView {
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
                                                    <td>{f.file}</td>
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

#[component]
fn HotspotsSection() -> impl IntoView {
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
                                            <td>{hs.file}</td>
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
fn SymbolsSection() -> impl IntoView {
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
                                                        <td>{s.file.clone()}</td>
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

#[component]
fn App() -> impl IntoView {
    view! {
        <h1>"repowise dashboard"</h1>
        <p class="subtitle">"live server"</p>
        <OverviewSection />
        <HealthSection />
        <HotspotsSection />
        <DecisionsSection />
        <SymbolsSection />
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
