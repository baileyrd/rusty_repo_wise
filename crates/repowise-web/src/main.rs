//! Phase 0 of the #59/#65 live-dashboard pivot: a minimal Leptos CSR
//! app proving the server-plus-WASM-frontend architecture works end to
//! end -- it fetches `GET /api/overview` from `repowise-server` and
//! renders it. Every other view the static `repowise dashboard` page
//! already has (health, hotspots, decisions, symbols) is a later phase,
//! not built here.

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

async fn fetch_overview() -> Result<Overview, String> {
    let response = gloo_net::http::Request::get("/api/overview")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.ok() {
        return Err(format!("server returned {}", response.status()));
    }
    response.json::<Overview>().await.map_err(|e| e.to_string())
}

#[component]
fn App() -> impl IntoView {
    let overview = LocalResource::new(fetch_overview);

    view! {
        <h1>"repowise dashboard"</h1>
        <p class="subtitle">"live server, phase 0"</p>
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

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
