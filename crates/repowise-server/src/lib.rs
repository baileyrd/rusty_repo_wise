//! A live HTTP server for the dashboard — the #59/#65 "real dashboard
//! parity" pivot: an axum backend exposing indexed-repo data as JSON,
//! plus static-asset serving for a WASM frontend (`repowise-web`),
//! replacing the one-shot `repowise dashboard` static HTML page with a
//! long-running server an SPA can poll/query live.
//!
//! Phase 0 proved the architecture with `GET /api/overview` alone.
//! Phase 1 (this module now) adds the rest of the static dashboard's
//! views onto the same JSON-API shape: `/api/health`, `/api/hotspots`,
//! `/api/decisions`, `/api/symbols`. Still not full parity — instant
//! search, the dependency-graph view, and the chat/LLM views are later
//! phases, not done here.
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
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
}

#[derive(Serialize)]
struct SymbolDto {
    name: String,
    kind: String,
    file: String,
    start_line: usize,
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

async fn get_decisions(State(state): State<AppState>) -> Result<Json<Vec<DecisionDto>>, ApiError> {
    let index = RepoIndex::load(&state.root)?;
    let decisions = repowise_adr::mine(&index).unwrap_or_default();
    Ok(Json(
        decisions
            .into_iter()
            .map(|d| DecisionDto {
                id: d.id,
                title: d.title,
                status: d.status,
                superseded_by: d.superseded_by,
                linked_file_count: d.linked_files.len(),
            })
            .collect(),
    ))
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
        .route("/api/health", get(get_health))
        .route("/api/hotspots", get(get_hotspots))
        .route("/api/decisions", get(get_decisions))
        .route("/api/symbols", get(get_symbols))
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
        };
        index.save(root).unwrap();
        index
    }

    async fn get(root: PathBuf, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = app(root, None)
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
            serde_json::from_slice(&body).unwrap()
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
        assert_eq!(json, serde_json::json!([]));
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
}
