//! A live HTTP server for the dashboard — Phase 0 of the #59/#65 "real
//! dashboard parity" pivot: an axum backend exposing indexed-repo data
//! as JSON, plus static-asset serving for a WASM frontend
//! (`repowise-web`), replacing the one-shot `repowise dashboard` static
//! HTML page with a long-running server an SPA can poll/query live.
//!
//! Only `GET /api/overview` exists so far — proving the
//! server-plus-frontend architecture end to end, not full parity with
//! every view the static dashboard already has. Porting the rest
//! (health, hotspots, decisions, symbols) onto this same JSON-API shape
//! is the next phase, not done here.
//!
//! Requires a prior `repowise init`/`update`, same as every other
//! command that reads `.repowise/index.json`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use repowise_core::RepoIndex;
use serde::Serialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    root: Arc<PathBuf>,
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

impl From<&repowise_graph::Overview> for OverviewDto {
    fn from(o: &repowise_graph::Overview) -> Self {
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
                .map(|(path, count)| (path.display().to_string(), *count))
                .collect(),
        }
    }
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
    Ok(Json(OverviewDto::from(&overview)))
}

/// Build the axum `Router` — separated from `serve` so tests can drive
/// requests directly against it (via `tower::ServiceExt::oneshot`)
/// without binding a real socket. `static_dir`, if given, serves the
/// built `repowise-web` frontend (e.g. `crates/repowise-web/dist` after
/// `trunk build`) as a fallback for any path the JSON API doesn't claim.
pub fn app(root: PathBuf, static_dir: Option<PathBuf>) -> Router {
    let state = AppState {
        root: Arc::new(root),
    };
    let router = Router::new()
        .route("/api/overview", get(get_overview))
        .with_state(state);
    match static_dir {
        Some(dir) => router.fallback_service(ServeDir::new(dir)),
        None => router,
    }
}

/// Bind `addr` and serve `app(root, static_dir)` until the process is
/// killed. `repowise-cli` drives this from a `tokio::runtime::Runtime`
/// it builds just for this command, the same "rest of the CLI stays
/// synchronous" pattern `repowise serve` (the MCP server) already uses.
pub async fn serve(
    root: PathBuf,
    addr: SocketAddr,
    static_dir: Option<PathBuf>,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app(root, static_dir)).await?;
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
        };
        index.save(&root).unwrap();

        let response = app(root, None)
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

        let response = app(root, None)
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
}
