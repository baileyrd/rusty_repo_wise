//! A static-site dashboard: one self-contained HTML page rendered from
//! data the other layers already computed (overview, health, hotspots,
//! decisions). No server process, no JS build step — regenerate by
//! re-running `repowise dashboard` and open the file in a browser.
//!
//! Kept deliberately simple for this first pass: a single page, no
//! per-file drill-down or live search. See the README for what a
//! richer/live version would need.

mod render;

use repowise_core::RepoIndex;
use repowise_graph::RepoGraph;
use std::path::{Path, PathBuf};

const DASHBOARD_DIR: &str = "dashboard";
const DASHBOARD_FILE: &str = "index.html";

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

    let html = render::render(root, &overview, &health, hotspots.as_deref(), &decisions);

    let dashboard_dir = root.join(RepoIndex::INDEX_DIR).join(DASHBOARD_DIR);
    std::fs::create_dir_all(&dashboard_dir)?;
    let path = dashboard_dir.join(DASHBOARD_FILE);
    std::fs::write(&path, html)?;
    Ok(path)
}
