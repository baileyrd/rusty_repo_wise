//! An MCP (Model Context Protocol) server exposing this port's index,
//! dependency graph, and health scoring as agent-facing tools over
//! stdio, using the official `rmcp` SDK.
//!
//! Implements 3 of the original repowise's ~10 MCP tools —
//! `get_overview`, `search_codebase`, `get_context` — the ones whose
//! backing data (the index, the resolved dependency graph, health
//! findings) already exists in this port. Tools like `get_risk`/
//! `get_change_risk` that the original scopes to this layer are left for
//! a follow-up: they'd read naturally on `repowise-git`'s hotspot data,
//! but wiring that in is a deliberate, separate addition rather than
//! bundled into this first pass.
//!
//! Every tool call re-loads `.repowise/index.json` and rebuilds the
//! dependency graph fresh — no in-memory caching across calls. Simple
//! and always-correct; if this ever needs to serve large repos with high
//! call volume, that's the first thing to revisit.

use repowise_core::{RepoIndex, SymbolKind};
use repowise_graph::RepoGraph;
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router,
    transport::stdio,
    ErrorData, ServiceExt,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Start the MCP server over stdio, indexing `root` (which must already
/// have a `.repowise/index.json` from a prior `repowise init`/`update`).
pub async fn run(root: PathBuf) -> anyhow::Result<()> {
    let server = RepowiseServer { root };
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[derive(Clone)]
struct RepowiseServer {
    root: PathBuf,
}

impl RepowiseServer {
    fn load(&self) -> Result<(RepoIndex, RepoGraph), ErrorData> {
        let index = RepoIndex::load(&self.root).map_err(|e| {
            ErrorData::internal_error(
                format!("failed to load index at {}: {e}", self.root.display()),
                None,
            )
        })?;
        let graph = RepoGraph::build(&index);
        Ok((index, graph))
    }

    fn resolve_file(&self, file: &str) -> PathBuf {
        let path = Path::new(file);
        let target = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        target.canonicalize().unwrap_or(target)
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct SearchParams {
    /// Case-insensitive substring to match against symbol names.
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct ContextParams {
    /// Path to the file, absolute or relative to the indexed root.
    file: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct LanguageCount {
    language: String,
    file_count: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct SymbolKindCount {
    kind: String,
    count: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct DependedOnFile {
    file: String,
    dependent_count: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct OverviewOutput {
    file_count: usize,
    other_file_count: usize,
    total_lines: usize,
    by_language: Vec<LanguageCount>,
    symbol_counts: Vec<SymbolKindCount>,
    import_edges: usize,
    call_edges: usize,
    unresolved_imports: usize,
    unresolved_calls: usize,
    most_depended_on: Vec<DependedOnFile>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct SymbolMatch {
    name: String,
    kind: String,
    file: String,
    line: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct SearchOutput {
    matches: Vec<SymbolMatch>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct HealthFindingOutput {
    kind: String,
    symbol: Option<String>,
    line: Option<usize>,
    detail: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct ContextOutput {
    file: String,
    symbols: Vec<SymbolMatch>,
    dependencies: Vec<String>,
    dependents: Vec<String>,
    health_score: f64,
    health_findings: Vec<HealthFindingOutput>,
}

fn display_rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[tool_router(server_handler)]
impl RepowiseServer {
    #[tool(
        name = "get_overview",
        description = "Summary stats about the indexed codebase: file/language/symbol counts, dependency-graph edge counts, and the most depended-on files. Requires a prior `repowise init`/`update`."
    )]
    fn get_overview(&self) -> Result<Json<OverviewOutput>, ErrorData> {
        let (index, graph) = self.load()?;
        let overview = graph.overview(&index);
        Ok(Json(OverviewOutput {
            file_count: overview.file_count,
            other_file_count: overview.other_file_count,
            total_lines: overview.total_lines,
            by_language: overview
                .by_language
                .into_iter()
                .map(|(language, file_count)| LanguageCount {
                    language,
                    file_count,
                })
                .collect(),
            symbol_counts: overview
                .symbol_counts
                .into_iter()
                .map(|(kind, count)| SymbolKindCount { kind, count })
                .collect(),
            import_edges: overview.import_edges,
            call_edges: overview.call_edges,
            unresolved_imports: overview.unresolved_imports,
            unresolved_calls: overview.unresolved_calls,
            most_depended_on: overview
                .most_depended_on
                .into_iter()
                .map(|(file, dependent_count)| DependedOnFile {
                    file: display_rel(&file, &index.root),
                    dependent_count,
                })
                .collect(),
        }))
    }

    #[tool(
        name = "search_codebase",
        description = "Case-insensitive substring search over indexed symbol names (functions, methods, classes, structs, etc.), returning each match's kind, file, and line number."
    )]
    fn search_codebase(
        &self,
        Parameters(SearchParams { query }): Parameters<SearchParams>,
    ) -> Result<Json<SearchOutput>, ErrorData> {
        if query.trim().is_empty() {
            return Err(ErrorData::invalid_params("query must not be empty", None));
        }
        let (index, graph) = self.load()?;
        let mut matches: Vec<SymbolMatch> = graph
            .search(&query)
            .into_iter()
            .map(|sym| SymbolMatch {
                name: sym.name.clone(),
                kind: sym.kind.label().to_string(),
                file: display_rel(&sym.file, &index.root),
                line: sym.start_line,
            })
            .collect();
        matches.sort_by(|a, b| a.name.cmp(&b.name).then(a.file.cmp(&b.file)));
        Ok(Json(SearchOutput { matches }))
    }

    #[tool(
        name = "get_context",
        description = "Complete context for one file in a single call: its symbols, resolved dependencies/dependents, and health findings/score. Built to replace the several separate reads (search, deps, health) an agent would otherwise need to piece this together itself."
    )]
    fn get_context(
        &self,
        Parameters(ContextParams { file }): Parameters<ContextParams>,
    ) -> Result<Json<ContextOutput>, ErrorData> {
        let (index, graph) = self.load()?;
        let target = self.resolve_file(&file);

        let Some(record) = index.files.iter().find(|f| f.path == target) else {
            return Err(ErrorData::resource_not_found(
                format!(
                    "{file} is not an indexed file under {}",
                    index.root.display()
                ),
                None,
            ));
        };

        let mut symbols: Vec<SymbolMatch> = record
            .symbols
            .iter()
            .filter(|s| !matches!(s.kind, SymbolKind::Module))
            .map(|sym| SymbolMatch {
                name: sym.name.clone(),
                kind: sym.kind.label().to_string(),
                file: display_rel(&sym.file, &index.root),
                line: sym.start_line,
            })
            .collect();
        symbols.sort_by_key(|s| s.line);

        let dependencies = graph
            .dependencies_of(&target)
            .into_iter()
            .map(|p| display_rel(&p, &index.root))
            .collect();
        let dependents = graph
            .dependents_of(&target)
            .into_iter()
            .map(|p| display_rel(&p, &index.root))
            .collect();

        let health = repowise_health::analyze(&index, &graph);
        let file_health = health
            .file_scores
            .iter()
            .find(|f| f.file == target)
            .map(|f| f.score)
            .unwrap_or(10.0);
        let health_findings = health
            .findings
            .iter()
            .filter(|f| f.file == target)
            .map(|f| HealthFindingOutput {
                kind: f.kind.label().to_string(),
                symbol: f.symbol.clone(),
                line: f.line,
                detail: f.detail.clone(),
            })
            .collect();

        Ok(Json(ContextOutput {
            file: display_rel(&target, &index.root),
            symbols,
            dependencies,
            dependents,
            health_score: file_health,
            health_findings,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{discover_files, FileRecord, Language};
    use rmcp::model::ErrorCode;

    /// Runs the real indexing pipeline (discover + parse) against real
    /// files on disk, then saves the index the tools load from — no
    /// hand-built fixtures standing in for what `repowise init` produces.
    fn build_and_save_index(root: &Path) {
        let discovered = discover_files(root).unwrap();
        let mut files: Vec<FileRecord> = Vec::new();
        let mut other_files = 0;
        for entry in discovered {
            if matches!(entry.language, Language::Other) {
                other_files += 1;
                continue;
            }
            let source = std::fs::read_to_string(&entry.path).unwrap();
            match repowise_parser::parse_file(&entry.path, entry.language, &source).unwrap() {
                Some(record) => files.push(record),
                None => other_files += 1,
            }
        }
        let index = RepoIndex {
            root: root.to_path_buf(),
            files,
            other_files,
        };
        index.save(root).unwrap();
    }

    #[test]
    fn get_overview_reports_file_and_symbol_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(overview) = server.get_overview().unwrap();
        assert_eq!(overview.file_count, 1);
        assert_eq!(
            overview
                .symbol_counts
                .iter()
                .find(|c| c.kind == "function")
                .unwrap()
                .count,
            1
        );
    }

    #[test]
    fn search_codebase_finds_symbols_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn HelperFunc() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(result) = server
            .search_codebase(Parameters(SearchParams {
                query: "helperfunc".to_string(),
            }))
            .unwrap();
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].name, "HelperFunc");
    }

    #[test]
    fn search_codebase_rejects_empty_query() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root };
        let result = server.search_codebase(Parameters(SearchParams {
            query: "  ".to_string(),
        }));
        let Err(err) = result else {
            panic!("expected an error for a blank query");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn get_context_returns_symbols_deps_and_health_for_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("lib.rs"),
            "mod util;\n\nfn caller() { util::helper(); }\n",
        )
        .unwrap();
        std::fs::write(root.join("util.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(ctx) = server
            .get_context(Parameters(ContextParams {
                file: "lib.rs".to_string(),
            }))
            .unwrap();
        assert_eq!(ctx.file, "lib.rs");
        assert!(ctx.symbols.iter().any(|s| s.name == "caller"));
        assert_eq!(ctx.dependencies, vec!["util.rs".to_string()]);
        // `caller` has no callers of its own, so it picks up a
        // possibly-dead-code finding (-0.2) — same heuristic the
        // repowise-health tests already establish.
        assert_eq!(ctx.health_score, 9.8);
        assert_eq!(ctx.health_findings.len(), 1);
    }

    #[test]
    fn get_context_errors_on_unindexed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root };
        let result = server.get_context(Parameters(ContextParams {
            file: "missing.rs".to_string(),
        }));
        let Err(err) = result else {
            panic!("expected an error for an unindexed file");
        };
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }
}
