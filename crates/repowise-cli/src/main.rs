mod indexing;

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
    },
    /// Generate a static HTML dashboard (overview, health, hotspots,
    /// decisions) under `.repowise/dashboard/index.html`.
    Dashboard {
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
        Command::Search { query, path } => cmd_search(&query, &path),
        Command::Deps { file, path } => cmd_deps(&file, &path),
        Command::Health { path, worst } => cmd_health(&path, worst),
        Command::Hotspots { path, top } => cmd_hotspots(&path, top),
        Command::Ownership { file, path } => cmd_ownership(&file, &path),
        Command::Coupled { file, path, top } => cmd_coupled(&file, &path, top),
        Command::Docs { path } => cmd_docs(&path),
        Command::Decisions { path, for_file } => cmd_decisions(&path, for_file.as_deref()),
        Command::Serve { path } => cmd_serve(&path),
        Command::Dashboard { path } => cmd_dashboard(&path),
    }
}

fn cmd_init(path: &Path) -> anyhow::Result<()> {
    let index = indexing::build_index(path)?;
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
    let index = indexing::build_index(&root)?;
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

fn cmd_health(path: &Path, worst: usize) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);
    let report = repowise_health::analyze(&index, &graph);

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

fn cmd_serve(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    // The rest of the CLI is synchronous; only the MCP server needs an
    // async runtime, so build one here rather than making `main` async.
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(repowise_mcp::run(root))
}

fn cmd_dashboard(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let written = repowise_dashboard::generate(&root)?;
    println!("Dashboard written to {}", written.display());
    Ok(())
}

fn display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
