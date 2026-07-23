//! A static-site dashboard: one self-contained HTML page rendered from
//! data the other layers already computed (overview, health, hotspots,
//! decisions). No server process, no JS build step — regenerate by
//! re-running `repowise dashboard` and open the file in a browser.
//!
//! Every file path rendered in the dashboard's tables links to that
//! file's `repowise-docs` wiki page when one already exists on disk
//! (i.e. `repowise docs` has been run at some point) — see
//! `wiki_page_for`'s doc comment for why "check disk, link if present"
//! was chosen over generating wiki pages as part of `dashboard` itself.
//! Kept deliberately simple otherwise: a single page, no live server, no
//! live search. See the README for what a richer/live version would need.

mod render;

use repowise_core::RepoIndex;
use repowise_graph::RepoGraph;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const DASHBOARD_DIR: &str = "dashboard";
const DASHBOARD_FILE: &str = "index.html";
// Must match `repowise_docs`'s own (private) `WIKI_DIR` constant. Not
// worth a cross-crate dependency just to share one string literal.
const WIKI_DIR: &str = "wiki";

/// Generate the dashboard for `root` (which must already have a
/// `.repowise/index.json` from a prior `repowise init`/`update`), writing
/// it to `<root>/.repowise/dashboard/index.html` and returning that path.
///
/// Git-history data (hotspots) and ADR/decision mining degrade
/// gracefully to "not available"/empty sections rather than failing the
/// whole call — neither is required for the dashboard's other sections.
pub fn generate(root: &Path) -> anyhow::Result<PathBuf> {
    let index = RepoIndex::load(root)?;
    let graph = RepoGraph::build(&index);
    let overview = graph.overview(&index);
    let health = repowise_health::analyze(&index, &graph);

    let hotspots = repowise_git::GitAnalytics::collect(root)
        .ok()
        .map(|analytics| repowise_git::hotspots(&index, &analytics));

    let decisions = repowise_adr::mine(&index).unwrap_or_default();

    let wiki_pages: HashSet<PathBuf> = index
        .files
        .iter()
        .map(|f| f.path.clone())
        .filter(|path| wiki_page_for(root, path).is_file())
        .collect();

    let html = render::render(
        root,
        &index,
        &overview,
        &health,
        hotspots.as_deref(),
        &decisions,
        &wiki_pages,
    );

    let dashboard_dir = root.join(RepoIndex::INDEX_DIR).join(DASHBOARD_DIR);
    std::fs::create_dir_all(&dashboard_dir)?;
    let path = dashboard_dir.join(DASHBOARD_FILE);
    std::fs::write(&path, html)?;
    Ok(path)
}

/// A file's `repowise-docs` wiki page path, whether or not it actually
/// exists on disk yet. `dashboard` deliberately doesn't generate wiki
/// pages itself (that would duplicate `repowise-docs`'s own freshness
/// tracking and re-read every file from disk on every dashboard build,
/// even when nothing changed) — it only checks whether `repowise docs`
/// has already produced one, and links to it if so. Running `repowise
/// docs` before `repowise dashboard` is what makes drill-down links
/// appear; this is documented in the README rather than enforced here.
fn wiki_page_for(root: &Path, file: &Path) -> PathBuf {
    let rel = file.strip_prefix(root).unwrap_or(file);
    root.join(RepoIndex::INDEX_DIR)
        .join(WIKI_DIR)
        .join(format!("{}.md", rel.display()))
}
