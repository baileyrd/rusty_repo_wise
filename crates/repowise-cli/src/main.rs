mod indexing;

use clap::{Parser, Subcommand};
use repowise_core::RepoIndex;
use repowise_graph::RepoGraph;
use std::path::{Path, PathBuf};

/// A Rust-native, self-hosted codebase intelligence CLI, inspired by
/// repowise (https://github.com/repowise-dev/repowise). This port focuses
/// on the dependency-graph intelligence layer: parsing, symbol/import/call
/// extraction, and graph queries. Other layers from the original project
/// (git analytics, doc generation, ADR mining, health scoring, MCP server,
/// dashboard) are out of scope for this phase.
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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => cmd_init(&path),
        Command::Update { path } => cmd_update(&path),
        Command::Overview { path } => cmd_overview(&path),
        Command::Search { query, path } => cmd_search(&query, &path),
        Command::Deps { file, path } => cmd_deps(&file, &path),
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

fn display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
