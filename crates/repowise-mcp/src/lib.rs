//! An MCP (Model Context Protocol) server exposing this port's index,
//! dependency graph, and health scoring as agent-facing tools over
//! stdio, using the official `rmcp` SDK.
//!
//! Implements 8 of the original repowise's ~10 MCP tools —
//! `get_overview`, `search_codebase`, `get_context`, `get_risk`,
//! `get_change_risk`, `get_symbol`, `get_why`, `get_dead_code` — the ones
//! whose backing data (the index, the resolved dependency graph, health
//! findings, `repowise-git`'s hotspot/churn/bug-fix and diff-shape data,
//! `repowise-adr`'s mined decisions, or raw source on disk) already
//! exists in this port. `get_change_risk`'s score is a documented
//! fixed-weight heuristic over diff-shape metrics (files/lines touched,
//! subsystems affected, change concentration, author experience) — the
//! original feeds the same kind of metrics into a pre-trained ML model,
//! which this port has no labeled corpus or training pipeline to
//! reproduce (see issue #42 and the category-A "ML-calibrated scoring"
//! issue). `get_dead_code`'s confidence tiers are likewise a documented
//! approximation of the original's model (which also folds in a
//! runtime-load risk factor — reflection, dynamic dispatch, entry
//! points — this port has no way to assess); see
//! `repowise_health::find_dead_code` for the exact tiering logic.
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

    /// Resolve a `get_why` target to a file path: if it exactly matches
    /// an indexed symbol's id, that symbol's own file; otherwise treated
    /// as a file path (same rules as `resolve_file`).
    fn resolve_target(&self, target: &str, index: &RepoIndex) -> PathBuf {
        index
            .files
            .iter()
            .flat_map(|f| &f.symbols)
            .find(|s| s.id == target)
            .map(|s| s.file.clone())
            .unwrap_or_else(|| self.resolve_file(target))
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

fn default_top_n() -> usize {
    10
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RiskParams {
    /// Path to a specific file to assess, absolute or relative to the
    /// indexed root. If omitted, returns the riskiest files repo-wide
    /// instead (ranked by hotspot score).
    #[serde(default)]
    file: Option<String>,
    /// How many files to return when `file` is omitted. Ignored when
    /// `file` is set (exactly one result either way).
    #[serde(default = "default_top_n")]
    top_n: usize,
}

impl Default for RiskParams {
    fn default() -> Self {
        RiskParams {
            file: None,
            top_n: default_top_n(),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct GetSymbolParams {
    /// A symbol's `id`, as returned by `search_codebase`/`get_context`.
    symbol_id: String,
    /// Extra lines of surrounding source to include on each side of the
    /// symbol's own line span, clamped to the file's bounds. Defaults to
    /// `0` (just the symbol's own span).
    #[serde(default)]
    context_lines: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct WhyParams {
    /// File paths (absolute or relative to the indexed root) or symbol
    /// ids (as returned by `search_codebase`/`get_context`) to filter
    /// mined decisions by. A decision matches if its body links to any
    /// target's file. Omit or leave empty to return every mined decision.
    #[serde(default)]
    targets: Vec<String>,
}

fn default_dead_code_limit() -> usize {
    50
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeadCodeParams {
    /// Minimum confidence tier to include: `"low"`, `"medium"`, or
    /// `"high"` (case-insensitive). Defaults to `"low"` (everything).
    /// Ignored when `safe_only` is set.
    #[serde(default)]
    min_confidence: Option<String>,
    /// When `true`, return only the `"high"` confidence tier — the
    /// closest this tool gets to the reference's "safe to delete"
    /// designation. Even so, this is a claim about this port's own
    /// resolution heuristics finding no in-repo reference, NOT a
    /// guarantee of runtime safety: reflection, dynamic dispatch, and
    /// entry points are all invisible to this port's static call graph.
    #[serde(default)]
    safe_only: bool,
    /// Maximum number of candidates to return. Defaults to 50.
    #[serde(default = "default_dead_code_limit")]
    limit: usize,
}

impl Default for DeadCodeParams {
    fn default() -> Self {
        DeadCodeParams {
            min_confidence: None,
            safe_only: false,
            limit: default_dead_code_limit(),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct ChangeRiskParams {
    /// A single commit, or a `base..head` range, to assess. Defaults to
    /// `HEAD` (the most recent commit) when omitted.
    #[serde(default)]
    revspec: Option<String>,
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
    /// Stable identifier for this symbol, usable with `get_symbol` to
    /// fetch its raw source text.
    id: String,
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

#[derive(Serialize, schemars::JsonSchema)]
struct FileRisk {
    file: String,
    /// churn × total cyclomatic complexity of the file's symbols (see
    /// `repowise_git::Hotspot`) — 0 for a file with no git history
    /// (unborn repo, uncommitted file, or `repowise-git` unavailable).
    hotspot_score: usize,
    /// Raw commit count touching this file. 0 under the same conditions
    /// as `hotspot_score`.
    churn: usize,
    /// Commits touching this file whose message matched a bug-fix
    /// keyword (see `repowise-git`). 0 under the same conditions.
    bugfix_commits: usize,
    health_score: f64,
    health_findings: Vec<HealthFindingOutput>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct RiskOutput {
    /// One entry when `file` was given in the request; up to `top_n`
    /// entries (highest hotspot score first) when it was omitted.
    files: Vec<FileRisk>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct ChangeRiskOutput {
    revspec: String,
    lines_added: usize,
    lines_deleted: usize,
    files_touched: usize,
    subsystems_touched: usize,
    /// `0.0..=1.0`; how evenly the changed lines are spread across the
    /// touched files (`0.0` = concentrated in one file, `1.0` = spread
    /// perfectly evenly). See `repowise_git::ChangeRisk` for the formula.
    concentration: f64,
    author: String,
    author_prior_commits: usize,
    /// `0.0..=10.0`, higher is riskier. A documented fixed-weight
    /// heuristic over the fields above — **not** a calibrated
    /// probability, and not the reference repowise's trained-model score
    /// (see the module doc comment).
    score: f64,
}

#[derive(Serialize, schemars::JsonSchema)]
struct GetSymbolOutput {
    id: String,
    name: String,
    kind: String,
    file: String,
    /// The returned `source`'s actual line span, after padding by
    /// `context_lines` and clamping to the file's bounds — not
    /// necessarily equal to the symbol's own `start_line..end_line`.
    start_line: usize,
    end_line: usize,
    source: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct DecisionOutput {
    id: String,
    title: String,
    /// `"adr:<file>"` or `"commit:<short hash> by <author>"`.
    source: String,
    /// Raw `Status:` line value (ADR source only).
    status: Option<String>,
    /// Normalized `ADR-XXXX` this decision is superseded by, if any.
    superseded_by: Option<String>,
    /// Raw `Date:` line value (ADR source only).
    date: Option<String>,
    linked_files: Vec<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct WhyOutput {
    decisions: Vec<DecisionOutput>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct DeadCodeCandidateOutput {
    file: String,
    symbol: String,
    line: usize,
    /// `"low"`, `"medium"`, or `"high"` — see the tool description and
    /// `repowise_health::find_dead_code` for the exact tiering logic.
    /// Not a runtime-safety guarantee at any tier.
    confidence: String,
    /// Why this candidate isn't `"high"` confidence (empty for `"high"`).
    risk_factors: Vec<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct DeadCodeOutput {
    candidates: Vec<DeadCodeCandidateOutput>,
    /// Total candidates matching the requested `min_confidence`/
    /// `safe_only` filter, before `limit` truncated the list — lets a
    /// caller tell "there were only 3" from "there were 300 and you're
    /// seeing the first 50".
    total_matching: usize,
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
                id: sym.id.clone(),
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
                id: sym.id.clone(),
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

    #[tool(
        name = "get_risk",
        description = "Risk assessment from git-history analytics and health findings, essentially `get_context` plus hotspot data. Given `file`, returns that file's hotspot score, churn, bug-fix-commit count, and health findings. Given no `file`, returns the `top_n` riskiest files repo-wide, ranked by (recency-weighted) hotspot score. Git data degrades to zero/empty when the indexed root isn't a git repository, rather than erroring."
    )]
    fn get_risk(
        &self,
        Parameters(RiskParams { file, top_n }): Parameters<RiskParams>,
    ) -> Result<Json<RiskOutput>, ErrorData> {
        let (index, graph) = self.load()?;
        let health = repowise_health::analyze(&index, &graph);
        // Not every indexed root is a git repository (or has git
        // available at all) — degrade to "no git data" rather than
        // failing the whole call, same tradeoff `repowise-dashboard`
        // already makes for its hotspots section.
        let analytics = repowise_git::GitAnalytics::collect(&self.root).ok();

        if let Some(file) = file {
            let target = self.resolve_file(&file);
            if !index.files.iter().any(|f| f.path == target) {
                return Err(ErrorData::resource_not_found(
                    format!(
                        "{file} is not an indexed file under {}",
                        index.root.display()
                    ),
                    None,
                ));
            }
            let risk = file_risk(&target, &index, analytics.as_ref(), &health);
            return Ok(Json(RiskOutput { files: vec![risk] }));
        }

        let files = analytics
            .as_ref()
            .map(|a| repowise_git::hotspots(&index, a))
            .unwrap_or_default()
            .into_iter()
            .take(top_n)
            .map(|h| file_risk(&h.file, &index, analytics.as_ref(), &health))
            .collect();
        Ok(Json(RiskOutput { files }))
    }

    #[tool(
        name = "get_change_risk",
        description = "Deterministic diff-shape risk score for a single commit or a `base..head` range: lines added/deleted, files touched, subsystems (top-level directories) touched, change concentration (how evenly the diff is spread across files), and the head commit's author's prior-commit count as an experience proxy. These combine into a documented fixed-weight 0-10 score. This is a heuristic approximation of the reference repowise's `get_change_risk`, NOT its ML-calibrated score — this port has no trained model or labeled defect corpus, so treat the number as a rough signal, not a probability."
    )]
    fn get_change_risk(
        &self,
        Parameters(ChangeRiskParams { revspec }): Parameters<ChangeRiskParams>,
    ) -> Result<Json<ChangeRiskOutput>, ErrorData> {
        let risk = repowise_git::change_risk(&self.root, revspec.as_deref()).map_err(|e| {
            ErrorData::invalid_params(format!("failed to compute change risk: {e}"), None)
        })?;
        Ok(Json(ChangeRiskOutput {
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

    #[tool(
        name = "get_symbol",
        description = "Raw source text for one indexed symbol by id (as returned by `search_codebase`/`get_context`), sliced from the symbol's own file at its `start_line..end_line` span. `context_lines` (default 0) pads that span by the same number of lines on each side, clamped to the file's actual bounds. Re-reads the file fresh from disk rather than trusting the index, so edits since the last `repowise init`/`update` are reflected (the returned span may then be off if line numbers have shifted)."
    )]
    fn get_symbol(
        &self,
        Parameters(GetSymbolParams {
            symbol_id,
            context_lines,
        }): Parameters<GetSymbolParams>,
    ) -> Result<Json<GetSymbolOutput>, ErrorData> {
        let (index, _graph) = self.load()?;

        let Some(sym) = index
            .files
            .iter()
            .flat_map(|f| &f.symbols)
            .find(|s| s.id == symbol_id)
        else {
            return Err(ErrorData::resource_not_found(
                format!("no indexed symbol with id {symbol_id}"),
                None,
            ));
        };

        let source = std::fs::read_to_string(&sym.file).map_err(|e| {
            ErrorData::internal_error(format!("failed to read {}: {e}", sym.file.display()), None)
        })?;
        let lines: Vec<&str> = source.lines().collect();

        // Clamp independently to the file's real (freshly re-read) line
        // count, then clamp `start_line` to never exceed `end_line` — the
        // file may have shrunk since this symbol was indexed.
        let end_line = (sym.end_line + context_lines).min(lines.len());
        let start_line = sym
            .start_line
            .saturating_sub(context_lines)
            .clamp(1, end_line.max(1));
        let snippet = lines[(start_line - 1)..end_line].join("\n");

        Ok(Json(GetSymbolOutput {
            id: sym.id.clone(),
            name: sym.name.clone(),
            kind: sym.kind.label().to_string(),
            file: display_rel(&sym.file, &index.root),
            start_line,
            end_line,
            source: snippet,
        }))
    }

    #[tool(
        name = "get_why",
        description = "Architectural decisions mined from docs/adr/*.md and decision-like commit messages (via repowise-adr), same data as `repowise decisions --for-file`. Given `targets` (file paths or symbol ids), returns only decisions whose body links to at least one target's file. Given no targets (or an empty list), returns every mined decision."
    )]
    fn get_why(
        &self,
        Parameters(WhyParams { targets }): Parameters<WhyParams>,
    ) -> Result<Json<WhyOutput>, ErrorData> {
        let (index, _graph) = self.load()?;
        let mut decisions = repowise_adr::mine(&index).map_err(|e| {
            ErrorData::internal_error(format!("failed to mine decisions: {e}"), None)
        })?;

        if !targets.is_empty() {
            let target_files: Vec<PathBuf> = targets
                .iter()
                .map(|t| self.resolve_target(t, &index))
                .collect();
            decisions.retain(|d| d.linked_files.iter().any(|f| target_files.contains(f)));
        }

        let decisions = decisions
            .into_iter()
            .map(|d| {
                let source = match &d.source {
                    repowise_adr::DecisionSource::Adr { file } => {
                        format!("adr:{}", display_rel(file, &index.root))
                    }
                    repowise_adr::DecisionSource::CommitMessage { hash, author } => {
                        format!("commit:{} by {author}", &hash[..hash.len().min(7)])
                    }
                    repowise_adr::DecisionSource::PullRequest { number, author } => {
                        format!("pr:{number} by {author}")
                    }
                };
                DecisionOutput {
                    id: d.id,
                    title: d.title,
                    source,
                    status: d.status,
                    superseded_by: d.superseded_by,
                    date: d.date,
                    linked_files: d
                        .linked_files
                        .iter()
                        .map(|f| display_rel(f, &index.root))
                        .collect(),
                }
            })
            .collect();

        Ok(Json(WhyOutput { decisions }))
    }

    #[tool(
        name = "get_dead_code",
        description = "Confidence-tiered dead-code candidates: functions/methods with zero resolved in-repo callers, tiered `low`/`medium`/`high` by how much two cheap risk factors (an ambiguous same-named symbol elsewhere, or an unresolved import that might have targeted this file) undercut that signal — see repowise_health::find_dead_code for the exact logic. `min_confidence` filters to that tier and above; `safe_only` narrows to `high` only, the closest this tool gets to the reference's 'safe to delete' designation. Even `high` confidence is a claim about this port's own static call graph, NOT a runtime-safety guarantee: reflection, dynamic dispatch, and entry points are invisible to it. `limit` caps the returned list (default 50); `total_matching` in the response reports how many matched before truncation."
    )]
    fn get_dead_code(
        &self,
        Parameters(DeadCodeParams {
            min_confidence,
            safe_only,
            limit,
        }): Parameters<DeadCodeParams>,
    ) -> Result<Json<DeadCodeOutput>, ErrorData> {
        let (index, graph) = self.load()?;
        let candidates = repowise_health::find_dead_code(&index, &graph);

        let threshold = if safe_only {
            repowise_health::DeadCodeConfidence::High
        } else {
            match min_confidence.as_deref() {
                None => repowise_health::DeadCodeConfidence::Low,
                Some(s) if s.eq_ignore_ascii_case("low") => {
                    repowise_health::DeadCodeConfidence::Low
                }
                Some(s) if s.eq_ignore_ascii_case("medium") => {
                    repowise_health::DeadCodeConfidence::Medium
                }
                Some(s) if s.eq_ignore_ascii_case("high") => {
                    repowise_health::DeadCodeConfidence::High
                }
                Some(other) => {
                    return Err(ErrorData::invalid_params(
                        format!("min_confidence must be low/medium/high, got {other:?}"),
                        None,
                    ));
                }
            }
        };

        let matching: Vec<_> = candidates
            .into_iter()
            .filter(|c| c.confidence >= threshold)
            .collect();
        let total_matching = matching.len();

        let candidates = matching
            .into_iter()
            .take(limit)
            .map(|c| DeadCodeCandidateOutput {
                file: display_rel(&c.file, &index.root),
                symbol: c.symbol,
                line: c.line,
                confidence: c.confidence.label().to_string(),
                risk_factors: c.risk_factors,
            })
            .collect();

        Ok(Json(DeadCodeOutput {
            candidates,
            total_matching,
        }))
    }
}

/// One file's risk profile: hotspot/churn/bug-fix data from `analytics`
/// (`None` when git data isn't available, reading as all-zero rather
/// than erroring) plus its health score/findings.
fn file_risk(
    file: &Path,
    index: &RepoIndex,
    analytics: Option<&repowise_git::GitAnalytics>,
    health: &repowise_health::HealthReport,
) -> FileRisk {
    let total_complexity: usize = index
        .files
        .iter()
        .find(|f| f.path == file)
        .map(|f| f.symbols.iter().map(|s| s.complexity).sum())
        .unwrap_or(0);
    let churn = analytics.map(|a| a.churn_of(file)).unwrap_or(0);
    let bugfix_commits = analytics.map(|a| a.bugfix_commits_of(file)).unwrap_or(0);
    let health_score = health
        .file_scores
        .iter()
        .find(|f| f.file == file)
        .map(|f| f.score)
        .unwrap_or(10.0);
    let health_findings = health
        .findings
        .iter()
        .filter(|f| f.file == file)
        .map(|f| HealthFindingOutput {
            kind: f.kind.label().to_string(),
            symbol: f.symbol.clone(),
            line: f.line,
            detail: f.detail.clone(),
        })
        .collect();

    FileRisk {
        file: display_rel(file, &index.root),
        hotspot_score: churn * total_complexity,
        churn,
        bugfix_commits,
        health_score,
        health_findings,
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

    /// Runs `git`, clearing the sandbox's own commit-identity env vars so
    /// they can't leak into these disposable test repos and override the
    /// local `user.name`/`user.email` set by `git_init`.
    fn git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_COMMITTER_NAME")
            .env_remove("GIT_COMMITTER_EMAIL")
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_init(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.name", "Default Author"]);
        git(dir, &["config", "user.email", "default@example.com"]);
    }

    #[test]
    fn get_risk_for_a_specific_file_reports_hotspot_and_health_data() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);

        std::fs::write(root.join("lib.rs"), "fn caller() { 1; }\n").unwrap();
        git(&root, &["add", "lib.rs"]);
        git(&root, &["commit", "-q", "-m", "Add lib"]);
        std::fs::write(
            root.join("lib.rs"),
            "fn caller() { 1; }\nfn caller2() { 2; }\n",
        )
        .unwrap();
        git(&root, &["commit", "-q", "-am", "Fix a bug in lib"]);

        build_and_save_index(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(risk) = server
            .get_risk(Parameters(RiskParams {
                file: Some("lib.rs".to_string()),
                top_n: 10,
            }))
            .unwrap();

        assert_eq!(risk.files.len(), 1);
        let file_risk = &risk.files[0];
        assert_eq!(file_risk.file, "lib.rs");
        assert_eq!(file_risk.churn, 2);
        assert_eq!(file_risk.bugfix_commits, 1);
        // hotspot_score = churn * total_complexity, and both functions
        // contribute complexity 1 each (no branches) -> 2 * 2 = 4.
        assert_eq!(file_risk.hotspot_score, 4);
        // Both functions are uncalled -> 2 possibly-dead-code findings.
        assert_eq!(file_risk.health_findings.len(), 2);
    }

    #[test]
    fn get_risk_without_a_file_returns_top_hotspots_repo_wide() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);

        std::fs::write(root.join("hot.rs"), "fn a() { 1; }\n").unwrap();
        std::fs::write(root.join("cold.rs"), "fn b() { 1; }\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "Add files"]);
        std::fs::write(root.join("hot.rs"), "fn a() { 1; }\nfn a2() { 2; }\n").unwrap();
        git(&root, &["commit", "-q", "-am", "Touch hot.rs again"]);

        build_and_save_index(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(risk) = server
            .get_risk(Parameters(RiskParams {
                file: None,
                top_n: 1,
            }))
            .unwrap();

        assert_eq!(risk.files.len(), 1);
        assert_eq!(risk.files[0].file, "hot.rs");
        assert_eq!(risk.files[0].churn, 2);
    }

    #[test]
    fn get_risk_degrades_gracefully_when_not_a_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "fn helper() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(risk) = server
            .get_risk(Parameters(RiskParams {
                file: Some("lib.rs".to_string()),
                top_n: 10,
            }))
            .unwrap();

        assert_eq!(risk.files.len(), 1);
        assert_eq!(risk.files[0].churn, 0);
        assert_eq!(risk.files[0].hotspot_score, 0);
    }

    #[test]
    fn get_risk_errors_on_unindexed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root };
        let result = server.get_risk(Parameters(RiskParams {
            file: Some("missing.rs".to_string()),
            top_n: 10,
        }));
        let Err(err) = result else {
            panic!("expected an error for an unindexed file");
        };
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }

    #[test]
    fn get_change_risk_defaults_to_head_and_reports_diff_shape() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);

        std::fs::write(root.join("lib.rs"), "fn a() {}\n").unwrap();
        git(&root, &["add", "lib.rs"]);
        git(&root, &["commit", "-q", "-m", "Add lib"]);
        std::fs::write(root.join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        git(&root, &["commit", "-q", "-am", "Grow lib"]);

        let server = RepowiseServer { root: root.clone() };
        let Json(risk) = server
            .get_change_risk(Parameters(ChangeRiskParams { revspec: None }))
            .unwrap();

        assert_eq!(risk.revspec, "HEAD");
        assert_eq!(risk.lines_added, 1);
        assert_eq!(risk.lines_deleted, 0);
        assert_eq!(risk.files_touched, 1);
        assert_eq!(risk.author, "default@example.com");
        assert!((0.0..=10.0).contains(&risk.score));
    }

    #[test]
    fn get_change_risk_accepts_an_explicit_range() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);

        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        git(&root, &["add", "a.txt"]);
        git(&root, &["commit", "-q", "-m", "Add a"]);
        git(&root, &["tag", "base"]);
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        git(&root, &["commit", "-q", "-am", "Grow a"]);

        let server = RepowiseServer { root: root.clone() };
        let Json(risk) = server
            .get_change_risk(Parameters(ChangeRiskParams {
                revspec: Some("base..HEAD".to_string()),
            }))
            .unwrap();

        assert_eq!(risk.revspec, "base..HEAD");
        assert_eq!(risk.lines_added, 1);
        assert_eq!(risk.files_touched, 1);
    }

    #[test]
    fn get_change_risk_errors_when_not_a_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let server = RepowiseServer { root };
        let result = server.get_change_risk(Parameters(ChangeRiskParams { revspec: None }));
        let Err(err) = result else {
            panic!("expected an error when the root isn't a git repository");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn get_symbol_returns_its_own_line_span_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("lib.rs"),
            "fn before() {}\n\nfn target() {\n    1\n}\n\nfn after() {}\n",
        )
        .unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(search) = server
            .search_codebase(Parameters(SearchParams {
                query: "target".to_string(),
            }))
            .unwrap();
        let symbol_id = search.matches[0].id.clone();

        let Json(sym) = server
            .get_symbol(Parameters(GetSymbolParams {
                symbol_id,
                context_lines: 0,
            }))
            .unwrap();

        assert_eq!(sym.name, "target");
        assert_eq!(sym.file, "lib.rs");
        assert_eq!(sym.start_line, 3);
        assert_eq!(sym.end_line, 5);
        assert_eq!(sym.source, "fn target() {\n    1\n}");
    }

    #[test]
    fn get_symbol_pads_and_clamps_context_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("lib.rs"),
            "fn before() {}\n\nfn target() {\n    1\n}\n\nfn after() {}\n",
        )
        .unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(search) = server
            .search_codebase(Parameters(SearchParams {
                query: "target".to_string(),
            }))
            .unwrap();
        let symbol_id = search.matches[0].id.clone();

        // Requesting far more context than the file has on either side
        // should clamp to the file's real bounds (lines 1..7) rather than
        // panicking or going out of range.
        let Json(sym) = server
            .get_symbol(Parameters(GetSymbolParams {
                symbol_id,
                context_lines: 100,
            }))
            .unwrap();

        assert_eq!(sym.start_line, 1);
        assert_eq!(sym.end_line, 7);
        assert_eq!(
            sym.source,
            "fn before() {}\n\nfn target() {\n    1\n}\n\nfn after() {}"
        );
    }

    #[test]
    fn get_symbol_errors_on_an_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root };
        let result = server.get_symbol(Parameters(GetSymbolParams {
            symbol_id: "nonexistent".to_string(),
            context_lines: 0,
        }));
        let Err(err) = result else {
            panic!("expected an error for an unknown symbol id");
        };
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }

    /// Two ADRs, each linking to a different indexed file via a mentioned
    /// symbol name (see `repowise_adr::link_to_index`) — no git repo
    /// needed, since ADR-file mining doesn't depend on commit history and
    /// `repowise_adr::mine` degrades commit-mining to empty when the root
    /// isn't a git repository.
    fn build_two_decision_fixture(root: &Path) {
        std::fs::create_dir_all(root.join("docs/adr")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/queue.rs"), "pub struct TaskQueue;\n").unwrap();
        std::fs::write(root.join("src/other.rs"), "pub struct OtherThing;\n").unwrap();
        std::fs::write(
            root.join("docs/adr/0001-queue.md"),
            "# ADR-0001: Use TaskQueue\n\nStatus: Accepted\nDate: 2026-01-01\n\n## Decision\nIntroduce TaskQueue for job scheduling.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("docs/adr/0002-other.md"),
            "# ADR-0002: Use OtherThing\n\nStatus: Accepted\nDate: 2026-02-01\n\n## Decision\nIntroduce OtherThing for config loading.\n",
        )
        .unwrap();
        build_and_save_index(root);
    }

    #[test]
    fn get_why_with_no_targets_returns_every_mined_decision() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);

        let server = RepowiseServer { root };
        let Json(why) = server
            .get_why(Parameters(WhyParams { targets: vec![] }))
            .unwrap();

        assert_eq!(why.decisions.len(), 2);
    }

    #[test]
    fn get_why_filters_by_file_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(why) = server
            .get_why(Parameters(WhyParams {
                targets: vec!["src/queue.rs".to_string()],
            }))
            .unwrap();

        assert_eq!(why.decisions.len(), 1);
        assert_eq!(why.decisions[0].title, "Use TaskQueue");
        assert_eq!(why.decisions[0].linked_files, vec!["src/queue.rs"]);
    }

    #[test]
    fn get_why_filters_by_symbol_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(search) = server
            .search_codebase(Parameters(SearchParams {
                query: "OtherThing".to_string(),
            }))
            .unwrap();
        let symbol_id = search.matches[0].id.clone();

        let Json(why) = server
            .get_why(Parameters(WhyParams {
                targets: vec![symbol_id],
            }))
            .unwrap();

        assert_eq!(why.decisions.len(), 1);
        assert_eq!(why.decisions[0].title, "Use OtherThing");
    }

    #[test]
    fn get_why_with_unmatched_target_returns_no_decisions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);

        let server = RepowiseServer { root };
        let Json(why) = server
            .get_why(Parameters(WhyParams {
                targets: vec!["src/nonexistent.rs".to_string()],
            }))
            .unwrap();

        assert_eq!(why.decisions.len(), 0);
    }

    #[test]
    fn get_dead_code_reports_high_confidence_for_an_uncalled_unambiguous_symbol() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("solo.rs"), "fn solo() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root };
        let Json(dead) = server
            .get_dead_code(Parameters(DeadCodeParams::default()))
            .unwrap();

        assert_eq!(dead.total_matching, 1);
        assert_eq!(dead.candidates.len(), 1);
        assert_eq!(dead.candidates[0].symbol, "solo");
        assert_eq!(dead.candidates[0].confidence, "high");
        assert!(dead.candidates[0].risk_factors.is_empty());
    }

    #[test]
    fn get_dead_code_safe_only_excludes_ambiguous_name_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("solo.rs"), "fn solo() {}\n").unwrap();
        std::fs::write(root.join("a.rs"), "fn dup() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn dup() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root: root.clone() };
        let Json(all) = server
            .get_dead_code(Parameters(DeadCodeParams::default()))
            .unwrap();
        assert_eq!(all.total_matching, 3);

        let Json(safe) = server
            .get_dead_code(Parameters(DeadCodeParams {
                min_confidence: None,
                safe_only: true,
                limit: 50,
            }))
            .unwrap();
        assert_eq!(safe.total_matching, 1);
        assert_eq!(safe.candidates[0].symbol, "solo");
        assert_eq!(safe.candidates[0].confidence, "high");
    }

    #[test]
    fn get_dead_code_limit_truncates_but_total_matching_reports_the_full_count() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "fn dup() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn dup() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root };
        let Json(dead) = server
            .get_dead_code(Parameters(DeadCodeParams {
                min_confidence: None,
                safe_only: false,
                limit: 1,
            }))
            .unwrap();

        assert_eq!(dead.candidates.len(), 1);
        assert_eq!(dead.total_matching, 2);
    }

    #[test]
    fn get_dead_code_rejects_an_invalid_min_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer { root };
        let result = server.get_dead_code(Parameters(DeadCodeParams {
            min_confidence: Some("extreme".to_string()),
            safe_only: false,
            limit: 50,
        }));
        let Err(err) = result else {
            panic!("expected an error for an invalid min_confidence");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }
}
