//! Deterministic, template-based per-file documentation ("wiki") pages —
//! no LLM involved. Each page is rendered entirely from data
//! `repowise-parser`/`repowise-graph`/`repowise-health` already computed:
//! a symbol list, resolved dependencies/dependents, and health findings
//! for that file.
//!
//! Freshness is tracked via a hash of the source file's own raw content,
//! embedded in each generated page and compared against the previous
//! run's page (if any). This is a per-file, own-content-only signal: a
//! page can be reported "unchanged" even though its rendered content
//! actually differs, if what changed was cross-file data (a new caller
//! elsewhere, a health finding driven by another file) rather than this
//! file's own source. Pages are always rewritten with current data
//! regardless of status — the status is a summary/reporting signal, not
//! a skip-rewrite optimization.

mod render;

use repowise_core::RepoIndex;
use repowise_graph::RepoGraph;
use repowise_health::HealthReport;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

const WIKI_DIR: &str = "wiki";
const HASH_MARKER: &str = "<!-- content-hash: ";

/// A file's wiki page path under `root`, whether or not it's actually
/// been generated yet -- exposed so other crates that augment (rather
/// than replace) this crate's deterministic pages, e.g. `repowise-llm`,
/// can locate them without duplicating the `.repowise/wiki/<rel>.md`
/// convention.
pub fn wiki_page_path(root: &Path, file: &Path) -> PathBuf {
    let rel = file.strip_prefix(root).unwrap_or(file);
    root.join(RepoIndex::INDEX_DIR)
        .join(WIKI_DIR)
        .join(format!("{}.md", rel.display()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageStatus {
    New,
    Changed,
    Unchanged,
}

#[derive(Debug, Clone)]
pub struct PageResult {
    pub source_file: PathBuf,
    pub wiki_path: PathBuf,
    pub status: PageStatus,
}

pub struct WikiSummary {
    pub pages: Vec<PageResult>,
}

impl WikiSummary {
    /// (new, changed, unchanged) counts.
    pub fn counts(&self) -> (usize, usize, usize) {
        let new = self
            .pages
            .iter()
            .filter(|p| p.status == PageStatus::New)
            .count();
        let changed = self
            .pages
            .iter()
            .filter(|p| p.status == PageStatus::Changed)
            .count();
        let unchanged = self
            .pages
            .iter()
            .filter(|p| p.status == PageStatus::Unchanged)
            .count();
        (new, changed, unchanged)
    }
}

/// Render and write one markdown page per indexed file under
/// `<root>/.repowise/wiki/`, mirroring each file's relative path with a
/// `.md` suffix appended (not substituted, so `foo.rs` and `foo.py`
/// can't collide on `foo.md`).
pub fn generate(
    index: &RepoIndex,
    graph: &RepoGraph,
    health: &HealthReport,
) -> anyhow::Result<WikiSummary> {
    let wiki_root = index.root.join(RepoIndex::INDEX_DIR).join(WIKI_DIR);
    std::fs::create_dir_all(&wiki_root)?;

    let mut findings_by_file: HashMap<PathBuf, Vec<&repowise_health::Finding>> = HashMap::new();
    for finding in &health.findings {
        findings_by_file
            .entry(finding.file.clone())
            .or_default()
            .push(finding);
    }
    let no_findings: Vec<&repowise_health::Finding> = Vec::new();

    let mut pages = Vec::new();
    for file in &index.files {
        let source = std::fs::read_to_string(&file.path).unwrap_or_default();
        let content_hash = hash_str(&source);
        let findings = findings_by_file.get(&file.path).unwrap_or(&no_findings);
        let page_content = render::render_page(file, content_hash, graph, &index.root, findings);

        let wiki_path = wiki_page_path(&index.root, &file.path);
        if let Some(parent) = wiki_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let previous_hash = std::fs::read_to_string(&wiki_path)
            .ok()
            .and_then(|s| parse_hash_marker(&s));
        let status = match previous_hash {
            None => PageStatus::New,
            Some(prev) if prev == content_hash => PageStatus::Unchanged,
            Some(_) => PageStatus::Changed,
        };

        std::fs::write(&wiki_path, &page_content)?;
        pages.push(PageResult {
            source_file: file.path.clone(),
            wiki_path,
            status,
        });
    }

    Ok(WikiSummary { pages })
}

fn parse_hash_marker(content: &str) -> Option<u64> {
    let line = content.lines().next()?;
    let rest = line.strip_prefix(HASH_MARKER)?;
    rest.trim_end_matches(" -->").trim().parse().ok()
}

fn hash_str(s: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}
