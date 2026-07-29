mod agent_md;
mod doctor;
mod export;
mod hook;
mod impacted;

use clap::{Parser, Subcommand};
use repowise_core::{RepoIndex, Symbol, SymbolKind};
use repowise_graph::RepoGraph;
use repowise_health::{DeadCodeCandidate, DeadCodeConfidence};
use std::path::{Path, PathBuf};

/// A Rust-native, self-hosted codebase intelligence CLI, inspired by
/// repowise (https://github.com/repowise-dev/repowise). Implemented so
/// far: parsing, symbol/import/call extraction, dependency-graph queries,
/// deterministic code-health scoring, git-history analytics (churn,
/// hotspots, ownership, co-change coupling), auto-generated per-file
/// documentation, architectural-decision mining, an MCP server exposing
/// a subset of these as agent-facing tools, and a static-site dashboard.
#[derive(Parser)]
#[command(name = "repowise", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a fresh index of a codebase.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Re-index a codebase (currently a full re-index; incremental
    /// re-indexing is not yet implemented).
    Update {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print summary stats about the indexed codebase.
    Overview {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Write a managed block of codebase intelligence into
    /// `CLAUDE.md` (default `.claude/CLAUDE.md`), preserving everything
    /// outside the markers. A file with no repowise markers is
    /// appended to, never rewritten; a file whose markers are malformed
    /// is refused rather than guessed at.
    GenerateClaudeMd {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Write somewhere other than `.claude/CLAUDE.md`. Use
        /// `AGENTS.md` for agents that read that instead.
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Print the generated block to stdout and write nothing.
        #[arg(long)]
        stdout: bool,
    },
    /// Search the index by symbol name (default), file path, or both.
    /// Case-insensitive substring match. `--mode semantic` is
    /// deliberately not offered -- it needs embeddings this port
    /// doesn't have (issue #61), and a silent fallback to substring
    /// matching would answer a different question than the one asked.
    Search {
        query: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        /// What to match against: `symbol` (default), `path`, or `hybrid`.
        #[arg(long, default_value = "symbol")]
        mode: String,
        /// Restrict to files of one role: implementation, test, config,
        /// doc, or unknown. Inferred from path conventions.
        #[arg(long)]
        kind: Option<String>,
        /// Restrict symbol hits to one kind (function, method, struct,
        /// enum, trait, class, module, mixin). Ignored in path mode.
        #[arg(long)]
        symbol_kind: Option<String>,
        /// Max results. 0 means no limit.
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// Show a file's resolved import dependencies and dependents.
    Deps {
        file: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Show deterministic code-health KPIs and the lowest-scoring files.
    Health {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many of the lowest-scoring files to list.
        #[arg(long, default_value_t = 10)]
        worst: usize,
        /// A (possibly partial) TOML file of per-marker penalty
        /// overrides -- an omitted key keeps its documented default.
        /// See `repowise_health::HealthWeights` for the field names.
        #[arg(long)]
        weights: Option<PathBuf>,
    },
    /// List confidence-tiered dead-code candidates: functions/methods
    /// with zero resolved in-repo callers.
    ///
    /// Even a `high`-confidence candidate is a claim about this port's
    /// own static call graph, not a runtime-safety guarantee --
    /// reflection, dynamic dispatch, and entry points are invisible to
    /// it. Review before deleting anything.
    DeadCode {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Minimum confidence tier to include: `low`, `medium`, or
        /// `high`. Mirrors the `get_dead_code` MCP tool's filter.
        #[arg(long, default_value = "low")]
        min_confidence: String,
        /// Cap the number of candidates listed; the total matching
        /// count is still reported.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Export the generated wiki, or the dependency graph as an
    /// architecture model.
    Export {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Directory to write into.
        #[arg(long)]
        out: PathBuf,
        /// `markdown` writes the wiki page tree; `json-graph` writes the
        /// dependency graph as a single JSON Graph Format document.
        #[arg(long, value_enum, default_value_t = ExportFormat::Markdown)]
        format: ExportFormat,
        /// Write into a non-empty target directory anyway.
        #[arg(long)]
        force: bool,
    },
    /// Ingest and inspect test-coverage reports (LCOV).
    Coverage {
        #[command(subcommand)]
        action: CoverageAction,
    },
    /// Print the tests a change provably exercises, by intersecting the
    /// diff's changed lines with the per-test coverage map.
    ///
    /// An empty list is NOT evidence that nothing is affected -- the
    /// output always states which of those two it is.
    ImpactedTests {
        /// A single commit or a `base..head` range. Defaults to `HEAD`.
        revspec: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Run setup diagnostics: git availability, history depth, index
    /// presence, and which optional env-var-gated features are active.
    ///
    /// Diagnostic only -- never mutates state. A degraded-but-working
    /// setup reports `warn`, not `fail`.
    Doctor {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Manage the post-commit git hook that refreshes the index after
    /// each commit.
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
    /// Report index freshness: whether an index exists, when it was
    /// written, and how much of it the working tree has moved past.
    ///
    /// Deliberately distinct from `overview`, which reports what's *in*
    /// the index -- this reports whether the index still describes the
    /// tree on disk.
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// List the individual stale/missing files rather than only
        /// counting them.
        #[arg(long)]
        verbose: bool,
    },
    /// Score the diff-shape risk of a commit or commit range.
    ///
    /// A documented fixed-weight heuristic over the shape of the diff
    /// (files, lines, subsystems, concentration, author experience) --
    /// NOT the reference repowise's ML-calibrated model. Treat the score
    /// as a rough approximation, not a calibrated probability.
    Risk {
        /// A single commit or a `base..head` range. Defaults to `HEAD`.
        revspec: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Rank files by hotspot score (git churn × cyclomatic complexity).
    Hotspots {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many of the highest-scoring files to list.
        #[arg(long, default_value_t = 15)]
        top: usize,
    },
    /// Show per-author line ownership for a file, from `git blame`.
    Ownership {
        file: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Show the files that most often change alongside a given file.
    Coupled {
        file: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many co-changing files to list.
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    /// Generate deterministic per-file documentation pages under
    /// `.repowise/wiki/`.
    Docs {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List mined architectural decisions (from docs/adr/*.md and
    /// decision-like commit messages), and which files they're linked to.
    Decisions {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Only show decisions linked to this file.
        #[arg(long)]
        for_file: Option<PathBuf>,
    },
    /// Run an MCP server over stdio exposing the agent-facing tools
    /// (get_overview, search_codebase, get_context, get_risk,
    /// get_change_risk, get_symbol, get_why, get_dead_code, get_health,
    /// and the
    /// workspace tools). Every response carries a `_meta` block with
    /// timing and index-staleness. Requires a prior `repowise
    /// init`/`update`.
    Serve {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Path to a workspace TOML file (see `repowise workspace-repos`
        /// and `repowise-workspace`'s own docs for the format) -- opts
        /// into the `list_repos` tool. Omit to run single-repo only.
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Generate a static HTML dashboard (overview, health, hotspots,
    /// decisions) under `.repowise/dashboard/index.html`.
    Dashboard {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Add an LLM-written summary to each existing wiki page under
    /// `.repowise/wiki/` (requires a prior `repowise docs`). Opt-in:
    /// needs `REPOWISE_LLM_BASE_URL` set to an OpenAI-compatible
    /// endpoint (e.g. a self-hosted rusty_provider instance).
    Generate {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Run a live dashboard server (JSON API + static frontend), the
    /// long-running replacement for the static `repowise dashboard`
    /// page. Phase 0: only `GET /api/overview` exists so far -- see
    /// `repowise-server`'s module doc comment.
    ServeDashboard {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: std::net::SocketAddr,
        /// Directory of the built `repowise-web` frontend (e.g.
        /// `crates/repowise-web/dist` after `trunk build`). Omit to
        /// run the JSON API alone, with no static frontend served.
        #[arg(long)]
        static_dir: Option<PathBuf>,
        /// Path to a workspace TOML file -- opts into `GET
        /// /api/workspace-repos` and the dashboard's Workspace section.
        /// Omit to run single-repo only.
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// List every repo configured in a workspace TOML file, each with
    /// its indexed status and file count if a prior `repowise init`/
    /// `update` has run there. See `repowise-workspace`'s own docs for
    /// the file format.
    WorkspaceRepos {
        #[arg(long)]
        workspace: PathBuf,
    },
    /// Show each workspace repo's own most-coupled file pairs (from its
    /// git history), side by side. Not cross-repo co-change -- separate
    /// repos have separate git histories -- just each repo's coupling
    /// shown together in one place. See `repowise-workspace`'s own docs
    /// for the file format.
    WorkspaceCoChanges {
        #[arg(long)]
        workspace: PathBuf,
        /// How many co-changing pairs to list per repo.
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    /// Resolve Rust `use` imports across workspace repo boundaries:
    /// which repos depend on which others, and the individual import
    /// sites behind each dependency. Rust-only -- see
    /// `repowise-workspace`'s own docs for why every other language's
    /// cross-repo imports are left unresolved.
    WorkspaceArchitecture {
        #[arg(long)]
        workspace: PathBuf,
    },
    /// Direct (one-hop, not transitive) cross-repo importers of one
    /// file in one workspace repo: which OTHER repos' files would need
    /// review if this file's public API changed.
    WorkspaceBlastRadius {
        #[arg(long)]
        workspace: PathBuf,
        /// Name of the workspace repo the target file lives in.
        #[arg(long)]
        repo: String,
        /// Path to the file within that repo, absolute or relative to
        /// that repo's own root.
        #[arg(long)]
        file: PathBuf,
    },
    /// Report circular cross-repo dependencies (repo A imports repo B
    /// imports repo A, or a longer cycle) -- reuses exactly the edges
    /// `workspace-architecture` already computes. A workspace's
    /// repo-level dependency graph should form a DAG; a cycle is a
    /// concrete, deterministic "pattern divergence" finding.
    WorkspaceConformance {
        #[arg(long)]
        workspace: PathBuf,
        /// Emit the raw conformance report as JSON.
        #[arg(long)]
        json: bool,
        /// Treat "nothing resolvable to check" as success. Off by
        /// default: a gate that can't see anything must not report a
        /// pass, so opting into that has to be explicit.
        #[arg(long)]
        allow_unverified: bool,
    },
    /// Regex-scan every workspace repo's indexed files for HTTP
    /// producer routes (axum/Flask/FastAPI/Express-style route
    /// registration) and consumer calls (fetch/axios/requests/ureq-
    /// style), matching consumer calls against producer routes in
    /// OTHER repos. Coarse and heuristic by design -- no cross-repo
    /// symbol resolution involved, just a fixed pattern table over raw
    /// source text. See `repowise-workspace`'s own docs for the caveat.
    /// Architecture-complexity metrics over the workspace's repo-level
    /// dependency graph: propagation cost (how far a change can reach),
    /// the cyclic core (which repos form circular dependencies), and a
    /// single deterministic 1-10 score. Structural edges only --
    /// co-change is excluded deliberately.
    WorkspaceMetrics {
        #[arg(long)]
        workspace: PathBuf,
        /// Emit the raw metrics as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Explain the cross-repo contract link count that
    /// `workspace-contracts` reports: per-repo producer/consumer
    /// counts, every unmatched consumer classified by WHY it didn't
    /// match, and producers nothing in the workspace calls. Answers
    /// the question a short contract list leaves open -- whether it's
    /// an architecture finding or just an unindexed repo.
    WorkspaceDiagnostics {
        #[arg(long)]
        workspace: PathBuf,
        /// Emit the raw diagnostics as JSON.
        #[arg(long)]
        json: bool,
    },
    WorkspaceContracts {
        #[arg(long)]
        workspace: PathBuf,
    },
}

/// What `repowise export` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ExportFormat {
    /// The generated wiki page tree, copied out verbatim.
    Markdown,
    /// The dependency graph as one JSON Graph Format document.
    JsonGraph,
}

/// `repowise coverage <action>`.
#[derive(Subcommand)]
enum CoverageAction {
    /// Ingest one or more LCOV reports, merging them into any coverage
    /// already recorded.
    Add {
        /// LCOV files to ingest.
        #[arg(required = true)]
        reports: Vec<PathBuf>,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Discard previously ingested coverage instead of merging into
        /// it.
        #[arg(long)]
        replace: bool,
    },
    /// Show the per-file coverage summary and per-test map statistics.
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many of the least-covered files to list.
        #[arg(long, default_value_t = 10)]
        worst: usize,
    },
}

/// `repowise hook <action>`. Split from `Command` so the three actions
/// share one `hook` namespace rather than three top-level commands.
#[derive(Subcommand)]
enum HookAction {
    /// Install the post-commit hook. Refuses to overwrite a hook this
    /// tool didn't write.
    Install {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Remove the post-commit hook. Refuses to remove a hook this tool
    /// didn't write.
    Uninstall {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Report whether the hook is installed, absent, or foreign.
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => cmd_init(&path),
        Command::Update { path } => cmd_update(&path),
        Command::Overview { path } => cmd_overview(&path),
        Command::GenerateClaudeMd {
            path,
            output,
            stdout,
        } => cmd_generate_claude_md(&path, output.as_deref(), stdout),
        Command::Search {
            query,
            path,
            mode,
            kind,
            symbol_kind,
            limit,
        } => cmd_search(
            &query,
            &path,
            &mode,
            kind.as_deref(),
            symbol_kind.as_deref(),
            limit,
        ),
        Command::Deps { file, path } => cmd_deps(&file, &path),
        Command::Health {
            path,
            worst,
            weights,
        } => cmd_health(&path, worst, weights.as_deref()),
        Command::DeadCode {
            path,
            min_confidence,
            limit,
        } => cmd_dead_code(&path, &min_confidence, limit),
        Command::Export {
            path,
            out,
            format,
            force,
        } => cmd_export(&path, &out, format, force),
        Command::Coverage { action } => cmd_coverage(action),
        Command::ImpactedTests { revspec, path } => cmd_impacted_tests(revspec.as_deref(), &path),
        Command::Doctor { path } => cmd_doctor(&path),
        Command::Hook { action } => cmd_hook(action),
        Command::Status { path, verbose } => cmd_status(&path, verbose),
        Command::Risk { revspec, path } => cmd_risk(revspec.as_deref(), &path),
        Command::Hotspots { path, top } => cmd_hotspots(&path, top),
        Command::Ownership { file, path } => cmd_ownership(&file, &path),
        Command::Coupled { file, path, top } => cmd_coupled(&file, &path, top),
        Command::Docs { path } => cmd_docs(&path),
        Command::Decisions { path, for_file } => cmd_decisions(&path, for_file.as_deref()),
        Command::Serve { path, workspace } => cmd_serve(&path, workspace),
        Command::Dashboard { path } => cmd_dashboard(&path),
        Command::Generate { path } => cmd_generate(&path),
        Command::ServeDashboard {
            path,
            addr,
            static_dir,
            workspace,
        } => cmd_serve_dashboard(&path, addr, static_dir, workspace),
        Command::WorkspaceMetrics { workspace, json } => cmd_workspace_metrics(&workspace, json),
        Command::WorkspaceDiagnostics { workspace, json } => {
            cmd_workspace_diagnostics(&workspace, json)
        }
        Command::WorkspaceRepos { workspace } => cmd_workspace_repos(&workspace),
        Command::WorkspaceCoChanges { workspace, top } => cmd_workspace_co_changes(&workspace, top),
        Command::WorkspaceArchitecture { workspace } => cmd_workspace_architecture(&workspace),
        Command::WorkspaceBlastRadius {
            workspace,
            repo,
            file,
        } => cmd_workspace_blast_radius(&workspace, &repo, &file),
        Command::WorkspaceConformance {
            workspace,
            json,
            allow_unverified,
        } => cmd_workspace_conformance(&workspace, json, allow_unverified),
        Command::WorkspaceContracts { workspace } => cmd_workspace_contracts(&workspace),
    }
}

/// Build an index and stamp it with the commit it describes.
///
/// The stamping happens here rather than in `repowise-parser` because
/// the parser has no git dependency by design. Both `init` and `update`
/// go through this, so the two can't disagree about whether an index
/// gets stamped — an unstamped index reads as "unknown commit" forever
/// downstream, and that difference should never come down to which
/// command someone happened to run.
fn build_stamped_index(root: &Path) -> anyhow::Result<RepoIndex> {
    let mut index = repowise_parser::build_index(root)?;
    index.indexed_commit = repowise_git::head_sha(&index.root);
    Ok(index)
}

fn cmd_init(path: &Path) -> anyhow::Result<()> {
    let index = build_stamped_index(path)?;
    let saved_to = index.save(&index.root)?;
    println!(
        "Indexed {} file(s) ({} other file(s) skipped) under {}",
        index.files.len(),
        index.other_files,
        index.root.display()
    );
    println!("Index written to {}", saved_to.display());
    Ok(())
}

fn cmd_update(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let previous = RepoIndex::load(&root).ok();
    let index = build_stamped_index(&root)?;
    let saved_to = index.save(&index.root)?;
    match previous {
        Some(prev) => {
            let delta = index.files.len() as i64 - prev.files.len() as i64;
            println!(
                "Updated index: {} file(s) indexed ({:+} vs previous run)",
                index.files.len(),
                delta
            );
        }
        None => {
            println!("No previous index found; created a new one.");
            println!("{} file(s) indexed", index.files.len());
        }
    }
    println!("Index written to {}", saved_to.display());
    Ok(())
}

fn cmd_overview(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);
    let overview = graph.overview(&index);

    println!("Repowise overview for {}", index.root.display());
    println!(
        "  {} indexed file(s), {} other file(s)",
        overview.file_count, overview.other_file_count
    );
    println!("  {} total lines", overview.total_lines);
    println!("  By language:");
    for (lang, count) in &overview.by_language {
        println!("    {lang:<10} {count}");
    }
    println!("  Symbols:");
    for (kind, count) in &overview.symbol_counts {
        println!("    {kind:<10} {count}");
    }
    println!(
        "  Edges: {} import(s), {} call(s) ({} unresolved import(s), {} unresolved call(s))",
        overview.import_edges,
        overview.call_edges,
        overview.unresolved_imports,
        overview.unresolved_calls
    );
    if !overview.most_depended_on.is_empty() {
        println!("  Most depended-on files:");
        for (file, count) in &overview.most_depended_on {
            println!("    {:<4} {}", count, display_path(file, &index.root));
        }
    }
    Ok(())
}

fn cmd_generate_claude_md(path: &Path, output: Option<&Path>, stdout: bool) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);
    let block = agent_md::render_block(&index, &graph, &root);

    if stdout {
        println!("{block}");
        return Ok(());
    }

    let target = match output {
        Some(p) if p.is_absolute() => p.to_path_buf(),
        Some(p) => root.join(p),
        None => root.join(agent_md::DEFAULT_OUTPUT),
    };

    let existing = std::fs::read_to_string(&target).ok();
    let (content, action) =
        agent_md::splice(existing.as_deref(), &block).map_err(|e| anyhow::anyhow!(e))?;

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, content)?;
    println!("{} {}", action.label(), target.display());
    Ok(())
}

/// Describe the filters that were applied, for the no-results message.
///
/// An empty result is ambiguous: "nothing in this repo matches" and
/// "your filters excluded everything" look identical, and the second is
/// the far more likely explanation once flags are in play. Echoing the
/// active filters back is what lets someone tell which they're looking
/// at without re-running the command to find out.
fn active_filters(
    mode: repowise_graph::SearchMode,
    kind: Option<repowise_graph::FileKind>,
    symbol_kind: Option<SymbolKind>,
) -> String {
    let mut parts = vec![format!("mode={}", mode.label())];
    if let Some(k) = kind {
        parts.push(format!("kind={}", k.label()));
    }
    if let Some(k) = symbol_kind {
        parts.push(format!("symbol_kind={}", k.label()));
    }
    parts.join(", ")
}

fn cmd_search(
    query: &str,
    path: &Path,
    mode: &str,
    kind: Option<&str>,
    symbol_kind: Option<&str>,
    limit: usize,
) -> anyhow::Result<()> {
    let mode = repowise_graph::SearchMode::parse(mode).map_err(|e| anyhow::anyhow!(e))?;
    let kind = kind
        .map(repowise_graph::FileKind::parse)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;
    let symbol_kind = symbol_kind
        .map(repowise_graph::parse_symbol_kind)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;

    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);

    // A file passes the `--kind` filter, or there is no filter.
    let file_allowed = |file: &Path| -> bool {
        let Some(want) = kind else { return true };
        index
            .files
            .iter()
            .find(|f| f.path == file)
            .map(|f| repowise_graph::classify(f, &index.root) == want)
            .unwrap_or(false)
    };

    let mut symbol_hits: Vec<&Symbol> = Vec::new();
    if matches!(
        mode,
        repowise_graph::SearchMode::Symbol | repowise_graph::SearchMode::Hybrid
    ) {
        symbol_hits = graph
            .search(query)
            .into_iter()
            .filter(|s| symbol_kind.is_none_or(|k| s.kind == k))
            .filter(|s| file_allowed(&s.file))
            .collect();
        symbol_hits.sort_by(|a, b| a.name.cmp(&b.name).then(a.file.cmp(&b.file)));
    }

    let mut path_hits: Vec<&Path> = Vec::new();
    if matches!(
        mode,
        repowise_graph::SearchMode::Path | repowise_graph::SearchMode::Hybrid
    ) {
        path_hits = index
            .files
            .iter()
            .filter(|f| repowise_graph::path_matches(&f.path, &index.root, query))
            .filter(|f| kind.is_none_or(|k| repowise_graph::classify(f, &index.root) == k))
            .map(|f| f.path.as_path())
            .collect();
        path_hits.sort();
    }

    let total = symbol_hits.len() + path_hits.len();
    if total == 0 {
        println!(
            "No matches for {query:?} ({})",
            active_filters(mode, kind, symbol_kind)
        );
        return Ok(());
    }

    let shown = if limit == 0 { total } else { limit.min(total) };
    let mut printed = 0usize;

    for sym in &symbol_hits {
        if printed >= shown {
            break;
        }
        println!(
            "{:<8} {:<30} {}:{}",
            sym.kind.label(),
            sym.name,
            display_path(&sym.file, &index.root),
            sym.start_line
        );
        printed += 1;
    }
    for file in &path_hits {
        if printed >= shown {
            break;
        }
        println!("{:<8} {}", "file", display_path(file, &index.root));
        printed += 1;
    }

    if printed < total {
        println!("... {} of {total} shown (--limit {limit})", printed);
    }
    Ok(())
}

fn cmd_deps(file: &Path, path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);

    let target = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let target = target.canonicalize().unwrap_or(target);

    let deps = graph.dependencies_of(&target);
    let dependents = graph.dependents_of(&target);

    println!("{}", display_path(&target, &index.root));
    println!("  depends on ({}):", deps.len());
    for d in &deps {
        println!("    {}", display_path(d, &index.root));
    }
    println!("  depended on by ({}):", dependents.len());
    for d in &dependents {
        println!("    {}", display_path(d, &index.root));
    }
    Ok(())
}

/// Upper bound on how many files count as "hot" when scoring
/// `hot_path_sync_io` (issue #186).
const HOT_PATH_MAX_FILES: usize = 10;

/// ...and the fraction of the repo that bound is additionally capped
/// to. Without this, a small repo has *every* file in its top 10, which
/// silently turns `hot_path_sync_io` into "any sync I/O anywhere" and
/// throws away the empirical half of the signal that justifies the
/// marker. A repo of 8 files gets a top 2; the cap only stops binding
/// once there are 40+ files.
const HOT_PATH_REPO_FRACTION: usize = 4;

/// The hottest files by hotspot score: the top
/// `HOT_PATH_MAX_FILES`, further capped to a
/// `1/HOT_PATH_REPO_FRACTION` slice of the repo, and never including a
/// file scoring zero (no churn or no complexity is not a hot path by
/// any definition). A relative rank rather than an absolute score
/// threshold, because hotspot scores are churn x complexity and so
/// aren't comparable between repos -- "the files this repo churns
/// hardest" travels; "score above 500" doesn't.
///
/// Returns an empty set when git history isn't available (no repo, a
/// shallow clone, git missing). Failing soft is deliberate:
/// `hot_path_sync_io` is the only marker that needs this, and losing one
/// marker is a much better outcome than refusing to score at all.
fn hot_path_files(root: &Path, index: &RepoIndex) -> std::collections::HashSet<PathBuf> {
    let Ok(analytics) = repowise_git::GitAnalytics::collect(root) else {
        return std::collections::HashSet::new();
    };
    let limit = HOT_PATH_MAX_FILES.min((index.files.len() / HOT_PATH_REPO_FRACTION).max(1));
    repowise_git::hotspots(index, &analytics)
        .into_iter()
        .filter(|h| h.score > 0)
        .take(limit)
        .map(|h| h.file)
        .collect()
}

fn cmd_health(path: &Path, worst: usize, weights_path: Option<&Path>) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);
    let weights = match weights_path {
        Some(p) => {
            let toml = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", p.display()))?;
            repowise_health::HealthWeights::from_toml_str(&toml)?
        }
        None => repowise_health::HealthWeights::default(),
    };
    let hot_files = hot_path_files(&root, &index);
    // Coverage is optional: without it the two coverage markers simply
    // never fire, rather than every file scoring as untested.
    let coverage = repowise_core::coverage::CoverageData::load(&root).ok();
    let report = repowise_health::analyze_with_context(
        &index,
        &graph,
        &weights,
        &hot_files,
        coverage.as_ref(),
    );

    println!("Repowise code health for {}", index.root.display());
    println!(
        "  average score: {:.1}/10 across {} file(s), {} marker(s) triggered",
        report.average_score,
        report.file_scores.len(),
        report.findings.len()
    );

    let by_kind = report.findings_by_kind();
    if !by_kind.is_empty() {
        println!("  markers by kind:");
        for (kind, count) in &by_kind {
            println!("    {:<20} {count}", kind.label());
        }
    }

    let worst_files: Vec<_> = report
        .file_scores
        .iter()
        .filter(|f| f.finding_count > 0)
        .take(worst)
        .collect();
    if !worst_files.is_empty() {
        println!("  lowest-scoring files:");
        for f in &worst_files {
            println!(
                "    {:<5.1} ({} marker(s))  {}",
                f.score,
                f.finding_count,
                display_path(&f.file, &index.root)
            );
        }
    }
    Ok(())
}

/// Parse the `--min-confidence` flag, mirroring the `get_dead_code` MCP
/// tool's accepted values so the two surfaces can't drift apart.
fn parse_min_confidence(raw: &str) -> anyhow::Result<DeadCodeConfidence> {
    match raw.to_ascii_lowercase().as_str() {
        "low" => Ok(DeadCodeConfidence::Low),
        "medium" => Ok(DeadCodeConfidence::Medium),
        "high" => Ok(DeadCodeConfidence::High),
        other => Err(anyhow::anyhow!(
            "min-confidence must be low/medium/high, got {other:?}"
        )),
    }
}

/// Render the dead-code listing. Split out from `cmd_dead_code` so the
/// filtering/truncation behaviour is testable without a real index on
/// disk -- the rest of the CLI's commands print inline, but this one has
/// enough logic between the analysis and the output to be worth pinning.
fn render_dead_code(
    candidates: Vec<DeadCodeCandidate>,
    root: &Path,
    threshold: DeadCodeConfidence,
    limit: usize,
) -> String {
    let matching: Vec<_> = candidates
        .into_iter()
        .filter(|c| c.confidence >= threshold)
        .collect();

    let mut out = String::new();
    out.push_str(&format!(
        "Repowise dead-code candidates for {}\n",
        root.display()
    ));

    if matching.is_empty() {
        out.push_str("  no candidates at or above the requested confidence tier\n");
        return out;
    }

    out.push_str(&format!(
        "  {} candidate(s) at or above `{}` confidence\n",
        matching.len(),
        threshold.label()
    ));

    for c in matching.iter().take(limit) {
        out.push_str(&format!(
            "    {:<7} {}:{}  {}\n",
            c.confidence.label(),
            display_path(&c.file, root),
            c.line,
            c.symbol
        ));
        for factor in &c.risk_factors {
            out.push_str(&format!("            - {factor}\n"));
        }
    }

    if matching.len() > limit {
        out.push_str(&format!(
            "  ... {} more not shown (raise --limit to see them)\n",
            matching.len() - limit
        ));
    }

    out.push_str("  Note: confidence describes this port's static call graph, not runtime\n");
    out.push_str("  safety. Reflection, dynamic dispatch, entry points, and #[test]\n");
    out.push_str("  functions are invisible to it -- review before deleting anything.\n");
    out
}

fn cmd_dead_code(path: &Path, min_confidence: &str, limit: usize) -> anyhow::Result<()> {
    let threshold = parse_min_confidence(min_confidence)?;
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);
    let candidates = repowise_health::find_dead_code(&index, &graph);

    print!(
        "{}",
        render_dead_code(candidates, &index.root, threshold, limit)
    );
    Ok(())
}

/// Index freshness, as reported by `repowise status`. Deliberately says
/// nothing about what the index *contains* -- that's `repowise
/// overview`'s job -- only about whether it still describes the tree on
/// disk.
#[derive(Debug, Default)]
struct StatusReport {
    /// `None` when no index exists yet: a valid state to report, not an
    /// error.
    indexed: Option<IndexedStatus>,
    /// Count of generated wiki pages under `.repowise/wiki`, if any.
    wiki_pages: usize,
    dashboard_present: bool,
}

#[derive(Debug)]
struct IndexedStatus {
    file_count: usize,
    /// Indexed files whose on-disk mtime is newer than the index itself.
    stale: Vec<PathBuf>,
    /// Indexed files that no longer exist on disk.
    missing: Vec<PathBuf>,
}

/// Compare each indexed file's mtime against the index's own mtime.
///
/// Deliberately filesystem-based rather than git-based: it works in a
/// repo with no git history, a shallow clone, or no git at all, and it
/// catches uncommitted edits that a `git diff` against the indexed
/// commit would miss. The tradeoff is that it can't see files that are
/// *new* since indexing -- finding those needs a full re-walk, which is
/// what `repowise update` does anyway.
fn collect_status(root: &Path) -> StatusReport {
    let mut report = StatusReport {
        wiki_pages: count_wiki_pages(root),
        dashboard_present: root
            .join(RepoIndex::INDEX_DIR)
            .join("dashboard")
            .join("index.html")
            .exists(),
        ..Default::default()
    };

    let index_path = RepoIndex::index_path(root);
    let Ok(index) = RepoIndex::load(root) else {
        return report;
    };
    let Some(index_mtime) = mtime_of(&index_path) else {
        return report;
    };

    let mut stale = Vec::new();
    let mut missing = Vec::new();
    for file in &index.files {
        match mtime_of(&file.path) {
            None => missing.push(file.path.clone()),
            Some(m) if m > index_mtime => stale.push(file.path.clone()),
            Some(_) => {}
        }
    }

    report.indexed = Some(IndexedStatus {
        file_count: index.files.len(),
        stale,
        missing,
    });
    report
}

fn mtime_of(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Count generated wiki pages.
///
/// Must recurse: `repowise docs` mirrors the repo's own tree under
/// `.repowise/wiki/`, so a repo whose sources live in subdirectories has
/// no pages at the wiki root at all. Shares `export`'s walk so the two
/// can't disagree about what counts as a page.
fn count_wiki_pages(root: &Path) -> usize {
    export::collect_pages(&repowise_docs::wiki_root(root)).len()
}

/// Render an index-freshness report. Pure, so every branch (no index,
/// clean, stale, missing files) is testable without touching disk.
fn render_status(report: &StatusReport, root: &Path, verbose: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!("Repowise status for {}\n", root.display()));

    match &report.indexed {
        None => {
            out.push_str("  index: none -- run `repowise init` to build one\n");
        }
        Some(idx) => {
            out.push_str(&format!("  index: {} file(s) indexed\n", idx.file_count));
            if idx.stale.is_empty() && idx.missing.is_empty() {
                out.push_str("  freshness: up to date\n");
            } else {
                out.push_str(&format!(
                    "  freshness: stale -- {} modified, {} missing since indexing\n",
                    idx.stale.len(),
                    idx.missing.len()
                ));
                if verbose {
                    for p in &idx.stale {
                        out.push_str(&format!("    modified  {}\n", display_path(p, root)));
                    }
                    for p in &idx.missing {
                        out.push_str(&format!("    missing   {}\n", display_path(p, root)));
                    }
                } else if idx.stale.len() + idx.missing.len() > 0 {
                    out.push_str("    (pass --verbose to list them)\n");
                }
                out.push_str("  run `repowise update` to re-index\n");
            }
            out.push_str(
                "  note: files created since indexing aren't detected here --\n\
                 \x20 finding those needs the full re-walk `repowise update` does.\n",
            );
        }
    }

    out.push_str(&format!(
        "  wiki: {}\n",
        match report.wiki_pages {
            0 => "no pages -- run `repowise docs`".to_string(),
            n => format!("{n} page(s) under .repowise/wiki"),
        }
    ));
    out.push_str(&format!(
        "  dashboard: {}\n",
        if report.dashboard_present {
            "generated"
        } else {
            "not generated -- run `repowise dashboard`"
        }
    ));
    out
}

fn cmd_export(path: &Path, out: &Path, format: ExportFormat, force: bool) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    match format {
        ExportFormat::Markdown => {
            let wiki_root = repowise_docs::wiki_root(&root);
            let plan = export::plan(&wiki_root, out, force)?;
            let count = export::execute(&plan)?;
            println!(
                "exported {count} wiki page(s) from {} to {}",
                wiki_root.display(),
                out.display()
            );
        }
        ExportFormat::JsonGraph => {
            let index = RepoIndex::load(&root)?;
            let graph = RepoGraph::build(&index);
            let doc = graph.to_json_graph(&index);
            let dest = export::json_graph_dest(out, force)?;
            // Pretty-printed rather than compact: an architecture model
            // is something people read and diff in review, and the size
            // difference doesn't matter for a file written once.
            std::fs::write(&dest, serde_json::to_string_pretty(&doc)?)?;
            println!(
                "exported {} node(s) and {} edge(s) to {}",
                doc.graph.nodes.len(),
                doc.graph.edges.len(),
                dest.display()
            );
            let unresolved = &doc.graph.metadata.unresolved;
            if unresolved.imports > 0 || unresolved.calls > 0 {
                // Say it out loud, not just in the file's metadata: the
                // graph is partial and a reader should know before they
                // draw conclusions from it.
                println!(
                    "  note: {} unresolved import(s) and {} unresolved call(s) have no",
                    unresolved.imports, unresolved.calls
                );
                println!("  edge (see graph.metadata.unresolved) -- absent edges do not");
                println!("  imply absent dependencies");
            }
        }
    }
    Ok(())
}

fn cmd_coverage(action: CoverageAction) -> anyhow::Result<()> {
    match action {
        CoverageAction::Add {
            reports,
            path,
            replace,
        } => cmd_coverage_add(&reports, &path, replace),
        CoverageAction::Status { path, worst } => cmd_coverage_status(&path, worst),
    }
}

fn cmd_coverage_add(reports: &[PathBuf], path: &Path, replace: bool) -> anyhow::Result<()> {
    use repowise_core::coverage::{self, CoverageData};

    let root = path.canonicalize()?;
    // Merge into existing coverage by default: the reference
    // auto-discovers and merges multiple sources, and a suite split
    // across CI shards produces one report per shard.
    let mut data = if replace {
        CoverageData::default()
    } else {
        CoverageData::load(&root).unwrap_or_default()
    };

    let mut total_unmatched = Vec::new();
    for report in reports {
        let text = std::fs::read_to_string(report)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", report.display()))?;
        let (parsed, summary) = coverage::ingest(&text, &root)
            .map_err(|e| anyhow::anyhow!("{}: {e}", report.display()))?;
        println!(
            "  {}: {} file(s), {} test context(s)",
            report.display(),
            summary.files_ingested,
            summary.tests_seen
        );
        total_unmatched.extend(summary.unmatched_paths);
        data.merge(parsed);
    }

    let saved = data.save(&root)?;
    println!(
        "ingested {} report(s) -> {} file(s) covered, {} test context(s) ({})",
        reports.len(),
        data.files.len(),
        data.per_test.len(),
        saved.display()
    );

    // Loud on purpose. Coverage whose paths don't line up with the repo
    // produces a clean-looking but empty result, which is the most
    // likely way this whole layer silently does nothing.
    if !total_unmatched.is_empty() {
        total_unmatched.sort();
        total_unmatched.dedup();
        println!(
            "  warning: {} path(s) in the report(s) matched no file under {}:",
            total_unmatched.len(),
            root.display()
        );
        for p in total_unmatched.iter().take(10) {
            println!("    {}", p.display());
        }
        if total_unmatched.len() > 10 {
            println!("    ... {} more", total_unmatched.len() - 10);
        }
    }
    Ok(())
}

fn cmd_coverage_status(path: &Path, worst: usize) -> anyhow::Result<()> {
    use repowise_core::coverage::CoverageData;

    let root = path.canonicalize()?;
    let Ok(data) = CoverageData::load(&root) else {
        println!("no coverage data -- run `repowise coverage add <REPORT>`");
        return Ok(());
    };

    let mut scored: Vec<(PathBuf, f64)> = data
        .files
        .keys()
        .filter_map(|p| data.line_coverage_of(p).map(|pct| (p.clone(), pct)))
        .collect();

    println!("Repowise coverage for {}", root.display());
    if scored.is_empty() {
        println!("  coverage recorded, but no file has any known lines");
        return Ok(());
    }

    let mean = scored.iter().map(|(_, p)| p).sum::<f64>() / scored.len() as f64;
    println!(
        "  {} file(s) measured, {:.1}% mean line coverage",
        scored.len(),
        mean
    );
    println!(
        "  per-test map: {}",
        if data.has_per_test_map() {
            format!("{} test context(s)", data.per_test.len())
        } else {
            "none -- reports carried no TN: records, so `impacted-tests` can't run".to_string()
        }
    );

    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    println!("  least-covered files:");
    for (file, pct) in scored.iter().take(worst) {
        println!("    {pct:>5.1}%  {}", display_path(file, &root));
    }
    Ok(())
}

fn cmd_impacted_tests(revspec: Option<&str>, path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let changed = repowise_git::changed_lines(&root, revspec)?;
    let coverage = repowise_core::coverage::CoverageData::load(&root).ok();
    let result = impacted::select(&changed, coverage.as_ref());
    print!(
        "{}",
        impacted::render(&result, revspec.unwrap_or("HEAD"), &root)
    );
    Ok(())
}

fn cmd_doctor(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let checks = doctor::run_checks(&root);
    print!("{}", doctor::render(&checks, &root));
    if doctor::any_failed(&checks) {
        // Warnings deliberately don't reach here: a degraded setup is
        // still a working one, and a nonzero exit would make `doctor`
        // useless in a CI gate that only cares about hard breakage.
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_hook(action: HookAction) -> anyhow::Result<()> {
    let message = match action {
        HookAction::Install { path } => hook::install(&path.canonicalize()?)?,
        HookAction::Uninstall { path } => hook::uninstall(&path.canonicalize()?)?,
        HookAction::Status { path } => hook::status(&path.canonicalize()?)?,
    };
    println!("{message}");
    Ok(())
}

fn cmd_status(path: &Path, verbose: bool) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let report = collect_status(&root);
    print!("{}", render_status(&report, &root, verbose));
    Ok(())
}

/// Bucket a 0-10 change-risk score into a word. Purely presentational --
/// the underlying score is a documented heuristic, not a calibrated
/// probability, so the bands are round numbers rather than thresholds
/// derived from any corpus.
fn risk_band(score: f64) -> &'static str {
    match score {
        s if s >= 7.0 => "high",
        s if s >= 4.0 => "moderate",
        _ => "low",
    }
}

/// Render a change-risk assessment. Split out from `cmd_risk` so the
/// formatting and banding are testable without a git repo present.
fn render_risk(risk: &repowise_git::ChangeRisk) -> String {
    let mut out = String::new();
    out.push_str(&format!("Repowise change risk for {}\n", risk.revspec));
    out.push_str(&format!(
        "  score: {:.1}/10 ({})\n",
        risk.score,
        risk_band(risk.score)
    ));
    out.push_str(&format!(
        "  diff shape: {} file(s), +{} / -{} line(s), {} subsystem(s)\n",
        risk.files_touched, risk.lines_added, risk.lines_deleted, risk.subsystems_touched
    ));
    out.push_str(&format!(
        "  concentration: {:.2} (0.00 = all in one file, 1.00 = spread evenly)\n",
        risk.concentration
    ));
    out.push_str(&format!(
        "  author: {} ({} prior commit(s) in this repo)\n",
        risk.author, risk.author_prior_commits
    ));
    out.push_str(
        "  Note: a fixed-weight diff-shape heuristic, not a calibrated\n\
         \x20 probability -- see repowise-git's change_risk docs for the formula.\n",
    );
    out
}

fn cmd_risk(revspec: Option<&str>, path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let risk = repowise_git::change_risk(&root, revspec)?;
    print!("{}", render_risk(&risk));
    Ok(())
}

fn cmd_hotspots(path: &Path, top: usize) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let analytics = repowise_git::GitAnalytics::collect(&root)?;
    let hotspots = repowise_git::hotspots(&index, &analytics);

    println!(
        "Repowise hotspots for {} ({} commit(s) analyzed)",
        index.root.display(),
        analytics.commit_count
    );
    if hotspots.is_empty() {
        println!("  No indexed file has git history under this root.");
        return Ok(());
    }
    println!(
        "  {:<10} {:<8} {:<6} {:<11} {:<8} {:<10} file (last touched by)",
        "score", "raw score", "churn", "complexity", "bugfixes", "last"
    );
    for h in hotspots.iter().take(top) {
        let last = h
            .last_touch
            .as_ref()
            .map(|(hash, author)| format!("{hash} {author}"))
            .unwrap_or_default();
        println!(
            "  {:<10.1} {:<8} {:<6} {:<11} {:<8} {:<10} {}",
            h.decayed_score,
            h.score,
            h.churn,
            h.total_complexity,
            h.bugfix_commits,
            last,
            display_path(&h.file, &index.root)
        );
    }
    Ok(())
}

fn cmd_ownership(file: &Path, path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let target = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let target = target.canonicalize().unwrap_or(target);

    let ownership = repowise_git::ownership_of(&root, &target)?;
    println!("{}", display_path(&target, &root));
    for o in &ownership {
        println!(
            "  {:>5.1}%  ({} line(s))  {}",
            o.percentage, o.lines, o.author
        );
    }
    println!(
        "{}",
        render_bus_factor(repowise_git::bus_factor(&ownership))
    );
    Ok(())
}

/// Phrase a bus factor so the number can't be read as a quality score.
/// A bare "bus factor: 1" invites the reading "one owner, tidy" -- the
/// opposite of what it means.
fn render_bus_factor(bus_factor: usize) -> String {
    match bus_factor {
        0 => "  bus factor: n/a (no blameable lines)".to_string(),
        1 => "  bus factor: 1 -- one author wrote most of this file".to_string(),
        n => format!("  bus factor: {n} -- {n} authors between them wrote most of this file"),
    }
}

fn cmd_coupled(file: &Path, path: &Path, top: usize) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let target = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let target = target.canonicalize().unwrap_or(target);

    let analytics = repowise_git::GitAnalytics::collect(&root)?;
    let coupled = analytics.coupled_files(&target, top);

    println!("{}", display_path(&target, &root));
    if coupled.is_empty() {
        println!("  No co-change coupling found (or too little history).");
        return Ok(());
    }
    println!("  Most often changed alongside:");
    for (f, count) in &coupled {
        println!("    {:<4} {}", count, display_path(f, &root));
    }
    Ok(())
}

fn cmd_docs(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);
    let health = repowise_health::analyze(&index, &graph);
    let summary = repowise_docs::generate(&index, &graph, &health)?;

    let (new, changed, unchanged) = summary.counts();
    println!(
        "Generated {} wiki page(s) under {}/.repowise/wiki",
        summary.pages.len(),
        index.root.display()
    );
    println!("  {new} new, {changed} changed, {unchanged} unchanged (by source content hash)");
    Ok(())
}

fn cmd_decisions(path: &Path, for_file: Option<&Path>) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let mut decisions = repowise_adr::mine(&index)?;

    if let Some(f) = for_file {
        let target = if f.is_absolute() {
            f.to_path_buf()
        } else {
            root.join(f)
        };
        let target = target.canonicalize().unwrap_or(target);
        decisions.retain(|d| d.linked_files.contains(&target));
    }

    println!(
        "Repowise decisions for {} ({} found)",
        index.root.display(),
        decisions.len()
    );
    if decisions.is_empty() {
        println!(
            "  No decisions found (docs/adr/*.md, decision-like commit messages, and merged PR bodies)."
        );
        return Ok(());
    }

    for d in &decisions {
        let source_label = match &d.source {
            repowise_adr::DecisionSource::Adr { file } => {
                format!("ADR ({})", display_path(file, &index.root))
            }
            repowise_adr::DecisionSource::CommitMessage { hash, author } => {
                format!("commit {} by {author}", &hash[..hash.len().min(7)])
            }
            repowise_adr::DecisionSource::PullRequest { number, author } => {
                format!("PR #{number} by {author}")
            }
            repowise_adr::DecisionSource::CodeComment { file, line } => {
                format!("comment ({}:{line})", display_path(file, &index.root))
            }
            repowise_adr::DecisionSource::InlineMarker { file, line, marker } => {
                format!(
                    "{marker} marker ({}:{line})",
                    display_path(file, &index.root)
                )
            }
            repowise_adr::DecisionSource::Changelog { file, section } => {
                format!("{section} (changelog: {})", display_path(file, &index.root))
            }
        };
        let status = d.status.as_deref().unwrap_or("-");
        println!("  {:<10} {:<10} {}", d.id, status, d.title);
        println!("    source: {source_label}");
        if let Some(target) = &d.superseded_by {
            println!("    superseded by: {target}");
        }
        if !d.linked_files.is_empty() {
            println!("    linked files:");
            for f in &d.linked_files {
                println!("      {}", display_path(f, &index.root));
            }
        }
    }
    Ok(())
}

fn cmd_serve(path: &Path, workspace: Option<PathBuf>) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    // The rest of the CLI is synchronous; only the MCP server needs an
    // async runtime, so build one here rather than making `main` async.
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(repowise_mcp::run(root, workspace))
}

fn cmd_serve_dashboard(
    path: &Path,
    addr: std::net::SocketAddr,
    static_dir: Option<PathBuf>,
    workspace: Option<PathBuf>,
) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let static_dir = static_dir.or_else(|| {
        let exe = std::env::current_exe().ok();
        let candidates = [
            PathBuf::from("web/dist"),
            PathBuf::from("C:\\dev\\rusty_repo_wise\\web\\dist"),
            exe.as_ref()
                .and_then(|p| p.parent().map(|d| d.join("web/dist")))
                .unwrap_or_default(),
            exe.as_ref()
                .and_then(|p| {
                    p.parent()
                        .and_then(|d| d.parent())
                        .and_then(|d| d.parent())
                        .map(|d| d.join("web/dist"))
                })
                .unwrap_or_default(),
            PathBuf::from("crates/repowise-web/dist"),
        ];
        candidates.into_iter().find(|p| p.exists() && p.is_dir())
    });

    if static_dir.is_none() {
        println!(
            "No --static-dir given and web/dist not found: serving the JSON API only (no frontend). \
             Build one with `cd web && npm run build` and pass \
             --static-dir web/dist."
        );
    } else if let Some(ref dir) = static_dir {
        println!("Serving static frontend from {}", dir.display());
    }
    println!("Dashboard server listening on http://{addr}");
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(repowise_server::serve(root, addr, static_dir, workspace))
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_repos(workspace: &Path) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    println!(
        "{} repo(s) configured in {}:",
        repos.len(),
        workspace.display()
    );
    for repo in &repos {
        let status = repowise_workspace::repo_status(repo);
        match status.file_count {
            Some(file_count) => println!(
                "  {} — {} ({file_count} file(s) indexed)",
                status.name,
                status.path.display()
            ),
            None => println!(
                "  {} — {} (not indexed; run `repowise init` there)",
                status.name,
                status.path.display()
            ),
        }
    }
    Ok(())
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_co_changes(workspace: &Path, top: usize) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    for report in repowise_workspace::workspace_co_changes(&repos, top) {
        println!("{} — {}", report.name, report.path.display());
        if !report.available {
            println!("  No git history found (or not a git repo).");
            continue;
        }
        if report.pairs.is_empty() {
            println!("  No co-change coupling found (or too little history).");
            continue;
        }
        for pair in &report.pairs {
            println!(
                "  {:<4} {} <-> {}",
                pair.count,
                display_path(&pair.file_a, &report.path),
                display_path(&pair.file_b, &report.path)
            );
        }
    }
    Ok(())
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_architecture(workspace: &Path) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let report = repowise_workspace::workspace_architecture(&repos);

    for status in &report.repos {
        let indexed = if status.indexed {
            format!("{} file(s) indexed", status.file_count.unwrap_or(0))
        } else {
            "not indexed".to_string()
        };
        println!("{} — {} ({indexed})", status.name, status.path.display());
    }

    if report.repo_edges.is_empty() {
        println!("\nNo cross-repo Rust imports resolved between the configured repos.");
        return Ok(());
    }

    println!("\nCross-repo dependencies:");
    for e in &report.repo_edges {
        println!(
            "  {} -> {} ({} edge(s))",
            e.from_repo, e.to_repo, e.edge_count
        );
    }

    println!("\nImport sites:");
    for e in &report.edges {
        println!(
            "  {} :: {} -> {} :: {} ({})",
            e.from_repo,
            e.from_file.display(),
            e.to_repo,
            e.to_file.display(),
            e.import_path
        );
    }
    Ok(())
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_blast_radius(workspace: &Path, repo: &str, file: &Path) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    let Some(target_repo) = repos.iter().find(|r| r.name == repo) else {
        anyhow::bail!("no repo named {repo:?} in {}", workspace.display());
    };

    let target_file = if file.is_absolute() {
        file.to_path_buf()
    } else {
        target_repo.path.join(file)
    };
    let target_file = target_file.canonicalize().unwrap_or(target_file);

    let importers = repowise_workspace::workspace_blast_radius(&repos, repo, &target_file);
    if importers.is_empty() {
        println!("No cross-repo importers found for this file.");
        return Ok(());
    }
    for e in &importers {
        println!(
            "{} :: {} ({})",
            e.from_repo,
            e.from_file.display(),
            e.import_path
        );
    }
    Ok(())
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
/// What a conformance run concluded, and whether it was in a position
/// to conclude anything.
///
/// The third variant is why this is an enum rather than a bool. "No
/// cycles found" and "no edges to check" produce the same empty cycle
/// list, and reporting both as a pass is a **false pass**: a Rust-only
/// resolver pointed at a Python workspace finds nothing every time, and
/// a CI gate that greens on that is worse than no gate, because it
/// looks like coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConformanceVerdict {
    /// Edges were resolved and no cycle exists among them. A real pass.
    Pass,
    /// At least one cycle. Fails the gate.
    Fail,
    /// Nothing resolvable to check. Neither a pass nor a failure.
    Unverified,
}

impl ConformanceVerdict {
    fn label(&self) -> &'static str {
        match self {
            ConformanceVerdict::Pass => "pass",
            ConformanceVerdict::Fail => "fail",
            ConformanceVerdict::Unverified => "unverified",
        }
    }

    /// Exit code for this verdict.
    ///
    /// `Unverified` exits non-zero alongside `Fail`. A gate that can't
    /// see anything must not report success -- the whole point of
    /// wiring this into CI is that a green run means something was
    /// actually checked. Someone who genuinely wants an unresolvable
    /// workspace to pass can say so with `--allow-unverified`, which
    /// makes that an explicit, reviewable choice rather than a silent
    /// default.
    fn exit_code(&self, allow_unverified: bool) -> i32 {
        match self {
            ConformanceVerdict::Pass => 0,
            ConformanceVerdict::Unverified if allow_unverified => 0,
            _ => 1,
        }
    }
}

/// Decide the verdict from the metrics the architecture pass already
/// computed.
///
/// Built on `workspace_metrics` rather than `detect_workspace_cycles`
/// alone so this command and `workspace-metrics` cannot disagree about
/// whether a workspace has cycles, and so the Rust-only resolvability
/// signal is shared rather than re-derived.
fn conformance_verdict(m: &repowise_workspace::WorkspaceMetrics) -> ConformanceVerdict {
    if !m.cyclic_core.is_empty() {
        return ConformanceVerdict::Fail;
    }
    if m.edge_count == 0 {
        return ConformanceVerdict::Unverified;
    }
    ConformanceVerdict::Pass
}

fn render_conformance(
    m: &repowise_workspace::WorkspaceMetrics,
    verdict: ConformanceVerdict,
) -> String {
    let mut out = String::new();

    if !m.unindexed_repos.is_empty() {
        out.push_str(&format!(
            "{} repo(s) could not be read and contributed no edges, so any cycle\n\
             through them is invisible to this check:\n",
            m.unindexed_repos.len()
        ));
        for name in &m.unindexed_repos {
            out.push_str(&format!("  {name} -- run `repowise update` in it\n"));
        }
        out.push('\n');
    }

    match verdict {
        ConformanceVerdict::Fail => {
            out.push_str(&format!(
                "FAIL: {} circular cross-repo dependency group(s) found.\n",
                m.cyclic_core.len()
            ));
            for cycle in &m.cyclic_core {
                let mut names = cycle.clone();
                names.sort();
                out.push_str(&format!("  {}\n", names.join(" <-> ")));
            }
            out.push_str("\nA workspace's repo-level dependency graph should form a DAG.\n");
        }
        ConformanceVerdict::Pass => {
            out.push_str(&format!(
                "PASS: {} cross-repo dependency edge(s) checked, no cycles.\n",
                m.edge_count
            ));
        }
        ConformanceVerdict::Unverified => {
            out.push_str(
                "UNVERIFIED: no cross-repo dependency edges were resolved, so there was\n\
                 nothing to check for cycles. This is NOT a pass -- an empty graph and a\n\
                 clean graph look identical here.\n",
            );
            out.push_str(&format!("  reason: {}\n", m.confidence.explanation()));
            out.push_str(
                "  Pass --allow-unverified to treat this as success in CI, deliberately.\n",
            );
        }
    }

    out.push_str(
        "\nCross-repo resolution is Rust-only; other languages' imports are left\n\
         unresolved, so a clean result bounds what was checked, not what exists.\n",
    );
    out
}

fn conformance_json(
    m: &repowise_workspace::WorkspaceMetrics,
    verdict: ConformanceVerdict,
) -> serde_json::Value {
    serde_json::json!({
        "verdict": verdict.label(),
        "cycles": m.cyclic_core,
        "edges_checked": m.edge_count,
        "repo_count": m.repo_count,
        "unindexed_repos": m.unindexed_repos,
        "confidence": {
            "level": m.confidence.label(),
            "explanation": m.confidence.explanation(),
        },
        "resolution_caveat":
            "cross-repo import resolution is Rust-only; a clean result bounds what was \
             checked, not what exists",
    })
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_conformance(
    workspace: &Path,
    json: bool,
    allow_unverified: bool,
) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let metrics = repowise_workspace::workspace_metrics(&repos);
    let verdict = conformance_verdict(&metrics);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&conformance_json(&metrics, verdict))?
        );
    } else {
        print!("{}", render_conformance(&metrics, verdict));
    }

    let code = verdict.exit_code(allow_unverified);
    if code != 0 {
        // Exit directly rather than returning Err: the report above is
        // the output, and anyhow would print a second "Error:" line
        // that adds nothing.
        std::process::exit(code);
    }
    Ok(())
}

/// Render the architecture-metrics report.
///
/// Split from `cmd_workspace_metrics` so the wording is testable
/// without a workspace on disk.
///
/// Leads with the confidence caveat whenever the graph wasn't fully
/// resolvable. Cross-repo resolution here is Rust-only, so a workspace
/// of Python services resolves zero edges and would otherwise be
/// reported as perfectly decoupled -- the best possible score for a
/// system nobody measured. That warning has to come before the numbers
/// it invalidates, not after.
fn render_metrics(m: &repowise_workspace::WorkspaceMetrics) -> String {
    use repowise_workspace::Confidence;
    let mut out = String::new();

    if m.confidence != Confidence::Resolved {
        out.push_str(&format!(
            "WARNING [{}]: {}\n\n",
            m.confidence.label(),
            m.confidence.explanation()
        ));
    }

    if !m.unindexed_repos.is_empty() {
        out.push_str(&format!(
            "{} repo(s) could not be read and contributed no edges in either\n\
             direction -- every number below is a floor, not a finding:\n",
            m.unindexed_repos.len()
        ));
        for name in &m.unindexed_repos {
            out.push_str(&format!("  {name} -- run `repowise update` in it\n"));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "Workspace architecture metrics\n  repos: {}\n  cross-repo dependency edges: {}\n",
        m.repo_count, m.edge_count
    ));
    out.push_str(&format!(
        "  propagation cost: {:.1}% (share of repo pairs where one can reach the other)\n",
        m.propagation_cost * 100.0
    ));

    match m.complexity_score {
        Some(score) => out.push_str(&format!(
            "  complexity score: {:.1}/10 ({})\n",
            score,
            repowise_workspace::WorkspaceMetrics::SCALE
        )),
        None => out.push_str(
            "  complexity score: not reported -- nothing measurable to score.\n\
             \x20   A score here would be the best possible number, earned by being\n\
             \x20   unmeasurable rather than by being well-structured.\n",
        ),
    }

    if m.cyclic_core.is_empty() {
        out.push_str("\nNo circular dependencies between repos.\n");
    } else {
        out.push_str(&format!(
            "\nCyclic core -- {} repo(s) in {} cycle(s):\n",
            m.repos_in_cyclic_core,
            m.cyclic_core.len()
        ));
        for cycle in &m.cyclic_core {
            let mut names = cycle.clone();
            names.sort();
            out.push_str(&format!("  {}\n", names.join(" <-> ")));
        }
    }

    out.push_str(
        "\nStructural edges only; co-change is excluded deliberately -- it moves with\n\
         how a team worked that quarter, and this score describes structure.\n\
         Cross-repo resolution is Rust-only: every other language's imports are\n\
         left unresolved, so these numbers are a lower bound on real coupling.\n",
    );

    out
}

fn metrics_json(m: &repowise_workspace::WorkspaceMetrics) -> serde_json::Value {
    serde_json::json!({
        "repo_count": m.repo_count,
        "edge_count": m.edge_count,
        "propagation_cost": m.propagation_cost,
        "complexity_score": m.complexity_score,
        "complexity_scale": repowise_workspace::WorkspaceMetrics::SCALE,
        "cyclic_core": m.cyclic_core,
        "repos_in_cyclic_core": m.repos_in_cyclic_core,
        "unindexed_repos": m.unindexed_repos,
        "confidence": {
            "level": m.confidence.label(),
            "explanation": m.confidence.explanation(),
        },
        "excludes": ["co-change (behavioral, not structural)"],
        "resolution_caveat":
            "cross-repo import resolution is Rust-only; other languages' imports are \
             unresolved, so these numbers are a lower bound on real coupling",
    })
}

fn cmd_workspace_metrics(workspace: &Path, json: bool) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let metrics = repowise_workspace::workspace_metrics(&repos);
    if json {
        println!("{}", serde_json::to_string_pretty(&metrics_json(&metrics))?);
    } else {
        print!("{}", render_metrics(&metrics));
    }
    Ok(())
}

/// Render the contract diagnostics report.
///
/// Split from `cmd_workspace_diagnostics` so the wording -- which is
/// most of this feature's value -- is testable without a workspace on
/// disk.
///
/// Leads with unindexed repos when there are any. That ordering is the
/// point of the whole command: every other number below is a floor
/// rather than a finding while a repo is missing from the scan, and
/// burying that under the counts would let someone read a confidently
/// wrong conclusion off the top of the output.
fn render_diagnostics(diag: &repowise_workspace::ContractDiagnostics) -> String {
    let mut out = String::new();
    let unindexed = diag.unindexed_repos();

    if !unindexed.is_empty() {
        out.push_str(&format!(
            "WARNING: {} of {} repo(s) could not be read, and contributed no routes\n\
             and no calls to anything below. Every count in this report is a floor,\n\
             not a finding, until these are indexed:\n",
            unindexed.len(),
            diag.repos.len()
        ));
        for name in &unindexed {
            out.push_str(&format!("  {name} -- run `repowise update` in it\n"));
        }
        out.push('\n');
    }

    out.push_str("Per-repo endpoints found:\n");
    for repo in &diag.repos {
        if repo.indexed {
            out.push_str(&format!(
                "  {:<24} {} producer route(s), {} consumer call(s)\n",
                repo.repo, repo.producers, repo.consumers
            ));
        } else {
            out.push_str(&format!("  {:<24} not indexed -- not scanned\n", repo.repo));
        }
    }

    out.push_str(&format!(
        "\nMatched cross-repo contracts: {}\n",
        diag.matches
    ));

    let by_reason = diag.unmatched_by_reason();
    if by_reason.is_empty() {
        out.push_str("\nEvery consumer call matched a producer in another repo.\n");
    } else {
        out.push_str("\nConsumer calls with no cross-repo contract, by reason:\n");
        for (reason, count) in by_reason {
            out.push_str(&format!(
                "  {:<22} {:>4}  -- {}\n",
                reason.label(),
                count,
                reason.explanation()
            ));
        }
        out.push_str("\nDetail:\n");
        for u in &diag.unmatched_consumers {
            out.push_str(&format!(
                "  [{}] {} ({} :: {})\n",
                u.reason.label(),
                u.call.path,
                u.call.repo,
                u.call.file.display()
            ));
        }
    }

    if diag.orphan_producers.is_empty() {
        out.push_str("\nNo orphan producer routes.\n");
    } else {
        out.push_str(&format!(
            "\nProducer routes nothing in this workspace calls ({}):\n\
             \x20 Either dead surface, or a consumer idiom this scan doesn't recognize --\n\
             \x20 this scan can't tell which, so neither is claimed.\n",
            diag.orphan_producers.len()
        ));
        for o in &diag.orphan_producers {
            out.push_str(&format!(
                "  {} ({} :: {})\n",
                o.route.path,
                o.route.repo,
                o.route.file.display()
            ));
        }
    }

    out
}

/// The report as JSON, hand-built rather than derived.
///
/// `ContractDiagnostics` deliberately doesn't derive `Serialize`:
/// `repowise-workspace` is a library crate with no serde dependency,
/// and the JSON shape here is a CLI output contract, which is a
/// different thing from the in-memory type and should be free to
/// diverge from it.
fn diagnostics_json(diag: &repowise_workspace::ContractDiagnostics) -> serde_json::Value {
    serde_json::json!({
        "repos": diag.repos.iter().map(|r| serde_json::json!({
            "repo": r.repo,
            "indexed": r.indexed,
            "producers": r.producers,
            "consumers": r.consumers,
        })).collect::<Vec<_>>(),
        "unindexed_repos": diag.unindexed_repos(),
        "matches": diag.matches,
        "unmatched_by_reason": diag.unmatched_by_reason().into_iter().map(|(r, n)| serde_json::json!({
            "reason": r.label(),
            "count": n,
            "explanation": r.explanation(),
        })).collect::<Vec<_>>(),
        "unmatched_consumers": diag.unmatched_consumers.iter().map(|u| serde_json::json!({
            "reason": u.reason.label(),
            "path": u.call.path,
            "repo": u.call.repo,
            "file": u.call.file.display().to_string(),
            "method": u.call.method,
        })).collect::<Vec<_>>(),
        "orphan_producers": diag.orphan_producers.iter().map(|o| serde_json::json!({
            "path": o.route.path,
            "repo": o.route.repo,
            "file": o.route.file.display().to_string(),
            "method": o.route.method,
        })).collect::<Vec<_>>(),
    })
}

fn cmd_workspace_diagnostics(workspace: &Path, json: bool) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let diag = repowise_workspace::workspace_diagnostics(&repos);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&diagnostics_json(&diag))?
        );
    } else {
        print!("{}", render_diagnostics(&diag));
    }
    Ok(())
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_contracts(workspace: &Path) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let report = repowise_workspace::workspace_contracts(&repos);

    if report.matches.is_empty() {
        println!("No cross-repo API contracts matched.");
    } else {
        println!("Matched cross-repo API contracts:");
        for m in &report.matches {
            println!(
                "  {}: {} ({}) <- {} ({})",
                m.path,
                m.producer_repo,
                m.producer_file.display(),
                m.consumer_repo,
                m.consumer_file.display()
            );
        }
    }

    if !report.unmatched_consumers.is_empty() {
        println!("\nConsumer calls with no known producer in this workspace:");
        for c in &report.unmatched_consumers {
            println!("  {} ({} :: {})", c.path, c.repo, c.file.display());
        }
    }
    Ok(())
}

fn cmd_dashboard(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let written = repowise_dashboard::generate(&root)?;
    println!("Dashboard written to {}", written.display());
    Ok(())
}

fn cmd_generate(path: &Path) -> anyhow::Result<()> {
    let Some(config) = repowise_llm::LlmConfig::from_env() else {
        anyhow::bail!(
            "LLM-assisted generation is opt-in and REPOWISE_LLM_BASE_URL isn't set. \
             Point it at an OpenAI-compatible endpoint (e.g. a self-hosted rusty_provider \
             instance) to enable it -- see README.md."
        );
    };
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;

    let results = repowise_llm::generate_wiki_summaries(&index, &config);
    let written = results
        .iter()
        .filter(|r| r.status == repowise_llm::SummaryStatus::Written)
        .count();
    let missing = results
        .iter()
        .filter(|r| r.status == repowise_llm::SummaryStatus::NoWikiPage)
        .count();
    let failed = results
        .iter()
        .filter(|r| r.status == repowise_llm::SummaryStatus::Failed)
        .count();

    println!(
        "Added LLM summaries to {written} wiki page(s) under {}/.repowise/wiki",
        index.root.display()
    );
    if missing > 0 {
        println!("  {missing} file(s) skipped: no wiki page yet -- run `repowise docs` first");
    }
    if failed > 0 {
        println!("  {failed} file(s) failed (LLM call or write error)");
    }
    Ok(())
}

fn display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_workspace::{
        ContractDiagnostics, OrphanProducer, RepoEndpointCounts, UnmatchedConsumer, UnmatchedReason,
    };

    fn candidate(
        name: &str,
        line: usize,
        confidence: DeadCodeConfidence,
        risk_factors: Vec<String>,
    ) -> DeadCodeCandidate {
        DeadCodeCandidate {
            file: PathBuf::from("/repo/src/lib.rs"),
            symbol: name.to_string(),
            line,
            confidence,
            risk_factors,
        }
    }

    #[test]
    fn parses_confidence_flag_case_insensitively() {
        assert_eq!(
            parse_min_confidence("HIGH").unwrap(),
            DeadCodeConfidence::High
        );
        assert_eq!(
            parse_min_confidence("medium").unwrap(),
            DeadCodeConfidence::Medium
        );
        assert!(parse_min_confidence("bogus").is_err());
    }

    #[test]
    fn renders_candidates_with_paths_relative_to_the_repo_root() {
        let out = render_dead_code(
            vec![candidate("unused_fn", 12, DeadCodeConfidence::High, vec![])],
            Path::new("/repo"),
            DeadCodeConfidence::Low,
            50,
        );
        assert!(out.contains("src/lib.rs:12"), "{out}");
        assert!(out.contains("unused_fn"), "{out}");
        assert!(out.contains("high"), "{out}");
    }

    #[test]
    fn filters_out_candidates_below_the_requested_tier() {
        let out = render_dead_code(
            vec![
                candidate(
                    "low_one",
                    1,
                    DeadCodeConfidence::Low,
                    vec!["ambiguous".into()],
                ),
                candidate("high_one", 2, DeadCodeConfidence::High, vec![]),
            ],
            Path::new("/repo"),
            DeadCodeConfidence::High,
            50,
        );
        assert!(out.contains("high_one"), "{out}");
        assert!(!out.contains("low_one"), "{out}");
        assert!(out.contains("1 candidate(s)"), "{out}");
    }

    #[test]
    fn reports_risk_factors_for_sub_high_candidates() {
        let out = render_dead_code(
            vec![candidate(
                "maybe_dead",
                3,
                DeadCodeConfidence::Medium,
                vec!["another symbol shares this name".into()],
            )],
            Path::new("/repo"),
            DeadCodeConfidence::Low,
            50,
        );
        assert!(out.contains("another symbol shares this name"), "{out}");
    }

    #[test]
    fn truncates_to_the_limit_but_still_reports_the_full_count() {
        let candidates: Vec<_> = (0..5)
            .map(|i| candidate(&format!("fn_{i}"), i, DeadCodeConfidence::High, vec![]))
            .collect();
        let out = render_dead_code(candidates, Path::new("/repo"), DeadCodeConfidence::Low, 2);
        assert!(out.contains("5 candidate(s)"), "{out}");
        assert!(out.contains("3 more not shown"), "{out}");
        assert!(out.contains("fn_1"), "{out}");
        assert!(!out.contains("fn_4"), "{out}");
    }

    #[test]
    fn empty_result_is_reported_as_a_clean_bill_not_an_error() {
        let out = render_dead_code(vec![], Path::new("/repo"), DeadCodeConfidence::High, 50);
        assert!(out.contains("no candidates"), "{out}");
    }

    fn risk(score: f64, files: usize) -> repowise_git::ChangeRisk {
        repowise_git::ChangeRisk {
            revspec: "HEAD".to_string(),
            lines_added: 40,
            lines_deleted: 5,
            files_touched: files,
            subsystems_touched: 2,
            concentration: 0.75,
            author: "Ada".to_string(),
            author_prior_commits: 12,
            score,
        }
    }

    #[test]
    fn bands_risk_scores_into_words() {
        assert_eq!(risk_band(9.0), "high");
        assert_eq!(risk_band(7.0), "high");
        assert_eq!(risk_band(5.5), "moderate");
        assert_eq!(risk_band(4.0), "moderate");
        assert_eq!(risk_band(1.2), "low");
    }

    #[test]
    fn renders_the_diff_shape_and_author_experience() {
        let out = render_risk(&risk(5.5, 3));
        assert!(out.contains("5.5/10"), "{out}");
        assert!(out.contains("moderate"), "{out}");
        assert!(out.contains("3 file(s)"), "{out}");
        assert!(out.contains("+40 / -5 line(s)"), "{out}");
        assert!(out.contains("Ada"), "{out}");
        assert!(out.contains("12 prior commit(s)"), "{out}");
    }

    #[test]
    fn states_that_the_score_is_a_heuristic_not_a_probability() {
        let out = render_risk(&risk(2.0, 1));
        assert!(out.contains("heuristic"), "{out}");
    }

    #[test]
    fn reports_the_revspec_it_actually_scored() {
        let mut r = risk(3.0, 1);
        r.revspec = "main..feature".to_string();
        let out = render_risk(&r);
        assert!(out.contains("main..feature"), "{out}");
    }

    #[test]
    fn status_reports_a_missing_index_as_a_state_not_an_error() {
        let report = StatusReport::default();
        let out = render_status(&report, Path::new("/repo"), false);
        assert!(out.contains("index: none"), "{out}");
        assert!(out.contains("repowise init"), "{out}");
    }

    #[test]
    fn status_reports_a_clean_index_as_up_to_date() {
        let report = StatusReport {
            indexed: Some(IndexedStatus {
                file_count: 7,
                stale: vec![],
                missing: vec![],
            }),
            wiki_pages: 3,
            dashboard_present: true,
        };
        let out = render_status(&report, Path::new("/repo"), false);
        assert!(out.contains("7 file(s) indexed"), "{out}");
        assert!(out.contains("up to date"), "{out}");
        assert!(out.contains("3 page(s)"), "{out}");
        assert!(out.contains("dashboard: generated"), "{out}");
    }

    #[test]
    fn status_counts_stale_and_missing_separately() {
        let report = StatusReport {
            indexed: Some(IndexedStatus {
                file_count: 5,
                stale: vec![PathBuf::from("/repo/a.rs"), PathBuf::from("/repo/b.rs")],
                missing: vec![PathBuf::from("/repo/gone.rs")],
            }),
            ..Default::default()
        };
        let out = render_status(&report, Path::new("/repo"), false);
        assert!(out.contains("2 modified, 1 missing"), "{out}");
        assert!(out.contains("repowise update"), "{out}");
        assert!(
            !out.contains("a.rs"),
            "non-verbose should not list files: {out}"
        );
    }

    #[test]
    fn status_verbose_lists_the_individual_files() {
        let report = StatusReport {
            indexed: Some(IndexedStatus {
                file_count: 5,
                stale: vec![PathBuf::from("/repo/a.rs")],
                missing: vec![PathBuf::from("/repo/gone.rs")],
            }),
            ..Default::default()
        };
        let out = render_status(&report, Path::new("/repo"), true);
        assert!(out.contains("modified  a.rs"), "{out}");
        assert!(out.contains("missing   gone.rs"), "{out}");
    }

    #[test]
    fn status_states_the_new_file_blind_spot() {
        let report = StatusReport {
            indexed: Some(IndexedStatus {
                file_count: 1,
                stale: vec![],
                missing: vec![],
            }),
            ..Default::default()
        };
        let out = render_status(&report, Path::new("/repo"), false);
        assert!(
            out.contains("created since indexing aren't detected"),
            "{out}"
        );
    }

    #[test]
    fn bus_factor_phrasing_cannot_be_read_as_a_quality_score() {
        // A bare "1" invites "one owner, tidy" -- the opposite of the meaning.
        let one = render_bus_factor(1);
        assert!(one.contains("one author wrote most"), "{one}");

        let three = render_bus_factor(3);
        assert!(three.contains("3 authors between them"), "{three}");

        let none = render_bus_factor(0);
        assert!(none.contains("n/a"), "{none}");
        assert!(none.contains("no blameable lines"), "{none}");
    }

    fn counts(repo: &str, producers: usize, consumers: usize, indexed: bool) -> RepoEndpointCounts {
        RepoEndpointCounts {
            repo: repo.to_string(),
            producers,
            consumers,
            indexed,
        }
    }

    fn call(repo: &str, path: &str) -> repowise_workspace::ConsumerCall {
        repowise_workspace::ConsumerCall {
            repo: repo.to_string(),
            file: PathBuf::from("app.js"),
            method: None,
            path: path.to_string(),
        }
    }

    /// An unindexed repo must be the FIRST thing the report says.
    /// Every count below it is a floor rather than a finding while a
    /// repo is missing from the scan, and burying that under the
    /// numbers is how someone reads a confidently wrong conclusion off
    /// the top of the output.
    #[test]
    fn diagnostics_leads_with_unread_repos() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 3, 0, true), counts("web", 0, 0, false)],
            matches: 0,
            unmatched_consumers: Vec::new(),
            orphan_producers: Vec::new(),
        };
        let out = render_diagnostics(&diag);
        let warn = out
            .find("WARNING")
            .expect("unread repos must be called out");
        let counts_at = out.find("Per-repo endpoints").unwrap();
        assert!(warn < counts_at, "the warning must come first:\n{out}");
        assert!(out.contains("floor"), "{out}");
        assert!(out.contains("repowise update"), "{out}");
        assert!(
            out.contains("web") && out.contains("not indexed -- not scanned"),
            "an unread repo's row must not read as '0 producers, 0 consumers':\n{out}"
        );
    }

    /// A clean workspace must not print the warning at all -- a banner
    /// that shows up unconditionally is one nobody reads.
    #[test]
    fn diagnostics_stays_quiet_when_every_repo_was_read() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 2, 0, true), counts("web", 0, 2, true)],
            matches: 2,
            unmatched_consumers: Vec::new(),
            orphan_producers: Vec::new(),
        };
        let out = render_diagnostics(&diag);
        assert!(!out.contains("WARNING"), "{out}");
        assert!(out.contains("Every consumer call matched"), "{out}");
        assert!(out.contains("No orphan producer routes"), "{out}");
    }

    #[test]
    fn diagnostics_groups_unmatched_consumers_by_reason_with_an_explanation() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 1, 2, true)],
            matches: 0,
            unmatched_consumers: vec![
                UnmatchedConsumer {
                    call: call("api", "/api/local"),
                    reason: UnmatchedReason::SameRepoOnly,
                },
                UnmatchedConsumer {
                    call: call("api", "/api/gone"),
                    reason: UnmatchedReason::NoProducerAnywhere,
                },
            ],
            orphan_producers: Vec::new(),
        };
        let out = render_diagnostics(&diag);
        assert!(out.contains("same-repo-only"), "{out}");
        assert!(out.contains("no-producer-anywhere"), "{out}");
        // The explanation is the point: a bare count of "2 unmatched"
        // is exactly the undifferentiated number this command exists
        // to break apart.
        assert!(out.contains("not a cross-repo contract"), "{out}");
        assert!(out.contains("pattern table doesn't recognize"), "{out}");
    }

    #[test]
    fn diagnostics_refuses_to_call_orphan_producers_dead() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 1, 0, true)],
            matches: 0,
            unmatched_consumers: Vec::new(),
            orphan_producers: vec![OrphanProducer {
                route: repowise_workspace::ProducerRoute {
                    repo: "api".to_string(),
                    file: PathBuf::from("routes.rs"),
                    method: None,
                    path: "/api/unused".to_string(),
                },
            }],
        };
        let out = render_diagnostics(&diag);
        assert!(out.contains("/api/unused"), "{out}");
        assert!(
            out.contains("Either dead surface, or a consumer idiom this scan doesn't recognize"),
            "an orphan route is ambiguous and the report must not resolve it:\n{out}"
        );
    }

    #[test]
    fn diagnostics_json_carries_the_reasons_not_just_the_counts() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 0, 1, true), counts("web", 0, 0, false)],
            matches: 0,
            unmatched_consumers: vec![UnmatchedConsumer {
                call: call("api", "/api/gone"),
                reason: UnmatchedReason::NoProducerAnywhere,
            }],
            orphan_producers: Vec::new(),
        };
        let v = diagnostics_json(&diag);
        assert_eq!(v["unindexed_repos"][0], "web");
        assert_eq!(
            v["unmatched_by_reason"][0]["reason"],
            "no-producer-anywhere"
        );
        assert!(v["unmatched_by_reason"][0]["explanation"]
            .as_str()
            .unwrap()
            .contains("external"));
        assert_eq!(v["unmatched_consumers"][0]["path"], "/api/gone");
    }

    fn metrics(
        confidence: repowise_workspace::Confidence,
        score: Option<f64>,
        cyclic: Vec<Vec<String>>,
    ) -> repowise_workspace::WorkspaceMetrics {
        repowise_workspace::WorkspaceMetrics {
            repo_count: 3,
            edge_count: 2,
            propagation_cost: 0.55,
            repos_in_cyclic_core: cyclic.iter().flatten().count(),
            cyclic_core: cyclic,
            complexity_score: score,
            unindexed_repos: Vec::new(),
            confidence,
        }
    }

    /// The whole reason the score is an Option. A workspace this port
    /// can't resolve must not be handed the best possible number for
    /// having been unmeasurable.
    #[test]
    fn metrics_withholds_the_score_when_nothing_was_measurable() {
        let out = render_metrics(&metrics(
            repowise_workspace::Confidence::NoResolvableLanguage,
            None,
            Vec::new(),
        ));
        assert!(out.contains("not reported"), "{out}");
        assert!(
            out.contains("unmeasurable rather than by being well-structured"),
            "{out}"
        );
        let warn = out.find("WARNING").expect("must warn");
        let numbers = out.find("Workspace architecture metrics").unwrap();
        assert!(
            warn < numbers,
            "the caveat must precede the numbers:\n{out}"
        );
    }

    #[test]
    fn metrics_states_the_scale_so_a_number_is_not_read_as_a_grade() {
        let out = render_metrics(&metrics(
            repowise_workspace::Confidence::Resolved,
            Some(4.2),
            Vec::new(),
        ));
        assert!(out.contains("4.2/10"), "{out}");
        assert!(out.contains("lower is better"), "{out}");
        assert!(
            !out.contains("WARNING"),
            "a resolved graph needs no caveat banner:\n{out}"
        );
    }

    #[test]
    fn metrics_always_states_what_was_excluded_and_what_could_not_resolve() {
        let out = render_metrics(&metrics(
            repowise_workspace::Confidence::Resolved,
            Some(2.0),
            Vec::new(),
        ));
        // Present even on the happy path: these bound what the number
        // means, and a reader who only ever sees clean output would
        // otherwise never learn the limits.
        assert!(out.contains("co-change is excluded"), "{out}");
        assert!(out.contains("Rust-only"), "{out}");
        assert!(out.contains("lower bound on real coupling"), "{out}");
    }

    #[test]
    fn metrics_lists_each_cycle() {
        let out = render_metrics(&metrics(
            repowise_workspace::Confidence::Resolved,
            Some(8.0),
            vec![vec!["b".to_string(), "a".to_string()]],
        ));
        assert!(out.contains("Cyclic core"), "{out}");
        assert!(
            out.contains("a <-> b"),
            "cycle members are sorted for stable output:\n{out}"
        );
    }

    #[test]
    fn metrics_json_carries_the_confidence_and_the_caveat() {
        let v = metrics_json(&metrics(
            repowise_workspace::Confidence::NoResolvableLanguage,
            None,
            Vec::new(),
        ));
        assert_eq!(v["confidence"]["level"], "no-resolvable-language");
        assert!(v["complexity_score"].is_null(), "withheld, not zero or ten");
        assert!(v["resolution_caveat"]
            .as_str()
            .unwrap()
            .contains("Rust-only"));
        assert_eq!(
            v["complexity_scale"],
            repowise_workspace::WorkspaceMetrics::SCALE
        );
    }

    fn conf_metrics(
        edges: usize,
        cycles: Vec<Vec<String>>,
        confidence: repowise_workspace::Confidence,
    ) -> repowise_workspace::WorkspaceMetrics {
        repowise_workspace::WorkspaceMetrics {
            repo_count: 3,
            edge_count: edges,
            propagation_cost: 0.5,
            repos_in_cyclic_core: cycles.iter().flatten().count(),
            cyclic_core: cycles,
            complexity_score: Some(2.0),
            unindexed_repos: Vec::new(),
            confidence,
        }
    }

    #[test]
    fn conformance_fails_on_a_cycle() {
        let m = conf_metrics(
            4,
            vec![vec!["b".to_string(), "a".to_string()]],
            repowise_workspace::Confidence::Resolved,
        );
        let v = conformance_verdict(&m);
        assert_eq!(v, ConformanceVerdict::Fail);
        assert_eq!(v.exit_code(false), 1, "a cycle must gate CI");
        assert_eq!(
            v.exit_code(true),
            1,
            "--allow-unverified must not excuse a real cycle"
        );
        let out = render_conformance(&m, v);
        assert!(out.contains("FAIL"), "{out}");
        assert!(out.contains("a <-> b"), "{out}");
    }

    #[test]
    fn conformance_passes_when_real_edges_were_checked() {
        let m = conf_metrics(4, Vec::new(), repowise_workspace::Confidence::Resolved);
        let v = conformance_verdict(&m);
        assert_eq!(v, ConformanceVerdict::Pass);
        assert_eq!(v.exit_code(false), 0);
        let out = render_conformance(&m, v);
        assert!(
            out.contains("4 cross-repo dependency edge(s) checked"),
            "a pass must say how much was checked, or it means nothing:\n{out}"
        );
    }

    /// The false pass this command exists to prevent. An empty graph
    /// and a clean graph produce the same empty cycle list, and a CI
    /// gate that greens on the former looks like coverage while
    /// providing none.
    #[test]
    fn conformance_will_not_call_an_unresolvable_workspace_a_pass() {
        let m = conf_metrics(
            0,
            Vec::new(),
            repowise_workspace::Confidence::NoResolvableLanguage,
        );
        let v = conformance_verdict(&m);
        assert_eq!(v, ConformanceVerdict::Unverified);
        assert_eq!(
            v.exit_code(false),
            1,
            "unverified must not exit 0 by default"
        );
        assert_eq!(v.exit_code(true), 0, "but it can be opted into explicitly");
        let out = render_conformance(&m, v);
        assert!(out.contains("UNVERIFIED"), "{out}");
        assert!(out.contains("NOT a pass"), "{out}");
        assert!(out.contains("--allow-unverified"), "{out}");
    }

    #[test]
    fn conformance_warns_that_unread_repos_hide_cycles() {
        let mut m = conf_metrics(2, Vec::new(), repowise_workspace::Confidence::Resolved);
        m.unindexed_repos = vec!["ghost".to_string()];
        let out = render_conformance(&m, conformance_verdict(&m));
        assert!(
            out.contains("any cycle\nthrough them is invisible"),
            "an unread repo can hide the very cycle this gate looks for:\n{out}"
        );
    }

    #[test]
    fn conformance_json_carries_the_verdict_and_what_was_checked() {
        let m = conf_metrics(
            0,
            Vec::new(),
            repowise_workspace::Confidence::NoResolvableLanguage,
        );
        let v = conformance_json(&m, conformance_verdict(&m));
        assert_eq!(v["verdict"], "unverified");
        assert_eq!(v["edges_checked"], 0);
        assert!(v["resolution_caveat"]
            .as_str()
            .unwrap()
            .contains("Rust-only"));
    }
}
