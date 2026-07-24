use clap::{Parser, Subcommand};
use repowise_core::RepoIndex;
use repowise_graph::RepoGraph;
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
    /// Search indexed symbols by name (case-insensitive substring match).
    Search {
        query: String,
        #[arg(default_value = ".")]
        path: PathBuf,
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
    /// Run an MCP server over stdio exposing get_overview/search_codebase/
    /// get_context. Requires a prior `repowise init`/`update`.
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
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => cmd_init(&path),
        Command::Update { path } => cmd_update(&path),
        Command::Overview { path } => cmd_overview(&path),
        Command::Search { query, path } => cmd_search(&query, &path),
        Command::Deps { file, path } => cmd_deps(&file, &path),
        Command::Health {
            path,
            worst,
            weights,
        } => cmd_health(&path, worst, weights.as_deref()),
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
        Command::WorkspaceRepos { workspace } => cmd_workspace_repos(&workspace),
        Command::WorkspaceCoChanges { workspace, top } => cmd_workspace_co_changes(&workspace, top),
        Command::WorkspaceArchitecture { workspace } => cmd_workspace_architecture(&workspace),
        Command::WorkspaceBlastRadius {
            workspace,
            repo,
            file,
        } => cmd_workspace_blast_radius(&workspace, &repo, &file),
        Command::WorkspaceConformance { workspace } => cmd_workspace_conformance(&workspace),
    }
}

fn cmd_init(path: &Path) -> anyhow::Result<()> {
    let index = repowise_parser::build_index(path)?;
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
    let index = repowise_parser::build_index(&root)?;
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

fn cmd_search(query: &str, path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);
    let mut matches = graph.search(query);
    matches.sort_by(|a, b| a.name.cmp(&b.name).then(a.file.cmp(&b.file)));

    if matches.is_empty() {
        println!("No symbols matching {query:?}");
        return Ok(());
    }
    for sym in matches {
        println!(
            "{:<8} {:<30} {}:{}",
            sym.kind.label(),
            sym.name,
            display_path(&sym.file, &index.root),
            sym.start_line
        );
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
    let report = repowise_health::analyze_with_weights(&index, &graph, &weights);

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
    Ok(())
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
    if static_dir.is_none() {
        println!(
            "No --static-dir given: serving the JSON API only (no frontend). \
             Build one with `cd crates/repowise-web && trunk build` and pass \
             --static-dir crates/repowise-web/dist."
        );
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
fn cmd_workspace_conformance(workspace: &Path) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let cycles = repowise_workspace::detect_workspace_cycles(&repos);
    if cycles.is_empty() {
        println!("No circular cross-repo dependencies found.");
        return Ok(());
    }
    println!("Circular cross-repo dependencies found:");
    for cycle in &cycles {
        println!("  {}", cycle.join(" <-> "));
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
