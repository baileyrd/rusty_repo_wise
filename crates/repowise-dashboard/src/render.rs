use repowise_adr::DecisionRecord;
use repowise_git::Hotspot;
use repowise_graph::Overview;
use repowise_health::HealthReport;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const STYLE: &str = r#"
:root { color-scheme: light dark; }
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  max-width: 960px;
  margin: 2rem auto;
  padding: 0 1rem;
  line-height: 1.5;
}
h1 { margin-bottom: 0.25rem; }
.subtitle { color: #767676; margin-top: 0; }
h2 { border-bottom: 1px solid #7676764d; padding-bottom: 0.25rem; margin-top: 2.5rem; }
table { border-collapse: collapse; width: 100%; margin: 1rem 0; }
th, td { text-align: left; padding: 0.35rem 0.6rem; border-bottom: 1px solid #7676764d; }
th { font-weight: 600; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
code, .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.9em; }
.empty { color: #767676; font-style: italic; }
.badge {
  display: inline-block;
  padding: 0.1rem 0.5rem;
  border-radius: 1rem;
  font-size: 0.8em;
  background: #7676761a;
}
"#;

/// Render the whole dashboard as one static HTML page from already-computed
/// data — no rendering logic reaches back out to disk or git itself.
/// `wiki_pages` says which indexed files already have a `repowise-docs`
/// wiki page on disk (computed by the caller, which does own the
/// filesystem check) — every file path rendered below links to its wiki
/// page when present in this set, or renders as plain (still-escaped)
/// text otherwise.
pub fn render(
    root: &Path,
    overview: &Overview,
    health: &HealthReport,
    hotspots: Option<&[Hotspot]>,
    decisions: &[DecisionRecord],
    wiki_pages: &HashSet<PathBuf>,
) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>repowise dashboard</title>\n<style>{STYLE}</style>\n</head>\n<body>\n\
         <h1>repowise dashboard</h1>\n\
         <p class=\"subtitle\">{}</p>\n\
         {}\n{}\n{}\n{}\n\
         </body>\n</html>\n",
        escape(&root.display().to_string()),
        overview_section(overview, root, wiki_pages),
        health_section(health, root, wiki_pages),
        hotspots_section(hotspots, root, wiki_pages),
        decisions_section(decisions),
    )
}

fn display_rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// A file path cell: a link to its wiki page (relative from
/// `.repowise/dashboard/index.html` to `.repowise/wiki/...`, the two
/// fixed sibling directories under `.repowise/`) when `wiki_pages`
/// contains it, plain escaped text otherwise — never a broken link.
fn file_cell(path: &Path, root: &Path, wiki_pages: &HashSet<PathBuf>) -> String {
    let rel = display_rel(path, root);
    if wiki_pages.contains(path) {
        format!(
            "<a href=\"../wiki/{}.md\">{}</a>",
            escape(&rel),
            escape(&rel)
        )
    } else {
        escape(&rel)
    }
}

fn overview_section(overview: &Overview, root: &Path, wiki_pages: &HashSet<PathBuf>) -> String {
    let mut out = String::from("<h2>Overview</h2>\n");
    out.push_str(&format!(
        "<p>{} indexed file(s), {} other file(s), {} total lines.</p>\n",
        overview.file_count, overview.other_file_count, overview.total_lines
    ));
    out.push_str("<table><tr><th>Language</th><th class=\"num\">Files</th></tr>\n");
    for (lang, count) in &overview.by_language {
        out.push_str(&format!(
            "<tr><td>{}</td><td class=\"num\">{count}</td></tr>\n",
            escape(lang)
        ));
    }
    out.push_str("</table>\n");

    out.push_str("<table><tr><th>Symbol kind</th><th class=\"num\">Count</th></tr>\n");
    for (kind, count) in &overview.symbol_counts {
        out.push_str(&format!(
            "<tr><td>{}</td><td class=\"num\">{count}</td></tr>\n",
            escape(kind)
        ));
    }
    out.push_str("</table>\n");

    out.push_str(&format!(
        "<p>{} import edge(s), {} call edge(s) ({} unresolved import(s), \
         {} unresolved call(s)).</p>\n",
        overview.import_edges,
        overview.call_edges,
        overview.unresolved_imports,
        overview.unresolved_calls
    ));

    if !overview.most_depended_on.is_empty() {
        out.push_str(
            "<h3>Most depended-on files</h3>\n<table><tr><th>File</th><th class=\"num\">Dependents</th></tr>\n",
        );
        for (file, count) in &overview.most_depended_on {
            out.push_str(&format!(
                "<tr><td class=\"mono\">{}</td><td class=\"num\">{count}</td></tr>\n",
                file_cell(file, root, wiki_pages)
            ));
        }
        out.push_str("</table>\n");
    }
    out
}

fn health_section(health: &HealthReport, root: &Path, wiki_pages: &HashSet<PathBuf>) -> String {
    let mut out = String::from("<h2>Code health</h2>\n");
    out.push_str(&format!(
        "<p>Average score: <strong>{:.1}/10</strong> across {} file(s), {} marker(s) triggered.</p>\n",
        health.average_score,
        health.file_scores.len(),
        health.findings.len()
    ));

    let by_kind = health.findings_by_kind();
    if !by_kind.is_empty() {
        out.push_str("<table><tr><th>Marker</th><th class=\"num\">Count</th></tr>\n");
        for (kind, count) in &by_kind {
            out.push_str(&format!(
                "<tr><td>{}</td><td class=\"num\">{count}</td></tr>\n",
                escape(kind.label())
            ));
        }
        out.push_str("</table>\n");
    }

    let worst: Vec<_> = health
        .file_scores
        .iter()
        .filter(|f| f.finding_count > 0)
        .take(15)
        .collect();
    if worst.is_empty() {
        out.push_str("<p class=\"empty\">No health findings.</p>\n");
    } else {
        out.push_str(
            "<h3>Lowest-scoring files</h3>\n\
             <table><tr><th>File</th><th class=\"num\">Score</th><th class=\"num\">Markers</th></tr>\n",
        );
        for f in worst {
            out.push_str(&format!(
                "<tr><td class=\"mono\">{}</td><td class=\"num\">{:.1}</td><td class=\"num\">{}</td></tr>\n",
                file_cell(&f.file, root, wiki_pages),
                f.score,
                f.finding_count
            ));
        }
        out.push_str("</table>\n");
    }
    out
}

fn hotspots_section(
    hotspots: Option<&[Hotspot]>,
    root: &Path,
    wiki_pages: &HashSet<PathBuf>,
) -> String {
    let mut out = String::from("<h2>Hotspots</h2>\n");
    let Some(hotspots) = hotspots else {
        out.push_str("<p class=\"empty\">No git history found under this root.</p>\n");
        return out;
    };
    if hotspots.is_empty() {
        out.push_str("<p class=\"empty\">No file has both git history and complexity.</p>\n");
        return out;
    }
    out.push_str(
        "<table><tr><th>File</th><th class=\"num\">Score (recency-weighted)</th>\
         <th class=\"num\">Score (raw)</th><th class=\"num\">Churn</th>\
         <th class=\"num\">Complexity</th><th class=\"num\">Bugfixes</th></tr>\n",
    );
    for h in hotspots.iter().take(15) {
        out.push_str(&format!(
            "<tr><td class=\"mono\">{}</td><td class=\"num\">{:.1}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td></tr>\n",
            file_cell(&h.file, root, wiki_pages),
            h.decayed_score,
            h.score,
            h.churn,
            h.total_complexity,
            h.bugfix_commits
        ));
    }
    out.push_str("</table>\n");
    out
}

fn decisions_section(decisions: &[DecisionRecord]) -> String {
    let mut out = String::from("<h2>Architectural decisions</h2>\n");
    if decisions.is_empty() {
        out.push_str(
            "<p class=\"empty\">No decisions found (docs/adr/*.md or decision-like commits).</p>\n",
        );
        return out;
    }
    out.push_str("<table><tr><th>ID</th><th>Title</th><th>Status</th><th class=\"num\">Linked files</th></tr>\n");
    for d in decisions {
        let status = match &d.superseded_by {
            Some(target) => format!("superseded by {}", escape(target)),
            None => d
                .status
                .as_deref()
                .map(escape)
                .unwrap_or_else(|| "<span class=\"badge\">commit</span>".to_string()),
        };
        out.push_str(&format!(
            "<tr><td class=\"mono\">{}</td><td>{}</td><td>{status}</td><td class=\"num\">{}</td></tr>\n",
            escape(&d.id),
            escape(&d.title),
            d.linked_files.len()
        ));
    }
    out.push_str("</table>\n");
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_adr::DecisionSource;
    use repowise_health::FindingKind;
    use std::path::PathBuf;

    #[test]
    fn escapes_untrusted_text() {
        assert_eq!(
            escape("<script>&\"x\"</script>"),
            "&lt;script&gt;&amp;&quot;x&quot;&lt;/script&gt;"
        );
    }

    #[test]
    fn renders_all_sections_with_relative_paths_and_placeholders_when_empty() {
        let root = PathBuf::from("/repo");
        let overview = Overview {
            file_count: 2,
            other_file_count: 1,
            by_language: vec![("Rust".to_string(), 2), ("<Weird>".to_string(), 1)],
            symbol_counts: vec![("function".to_string(), 3)],
            total_lines: 42,
            import_edges: 1,
            call_edges: 2,
            unresolved_imports: 0,
            unresolved_calls: 0,
            most_depended_on: vec![(root.join("core.rs"), 4)],
        };
        let health = repowise_health::HealthReport {
            file_scores: vec![repowise_health::FileHealth {
                file: root.join("messy.rs"),
                score: 6.5,
                finding_count: 2,
            }],
            findings: vec![repowise_health::Finding {
                file: root.join("messy.rs"),
                symbol: Some("<risky>".to_string()),
                line: Some(3),
                kind: FindingKind::HighComplexity,
                detail: "cyclomatic complexity 15 (> 10)".to_string(),
            }],
            average_score: 6.5,
        };

        let html = render(&root, &overview, &health, None, &[], &HashSet::new());

        // Relative paths, not absolute.
        assert!(html.contains(">core.rs<"));
        assert!(html.contains(">messy.rs<"));
        assert!(!html.contains("/repo/core.rs"));
        // No wiki pages exist (empty set) -- plain text, no dangling link.
        assert!(!html.contains("<a href"));
        // User-controlled text (e.g. a language label) is escaped.
        assert!(html.contains("&lt;Weird&gt;"));
        // No git history / no decisions render as explicit placeholders,
        // not silently blank sections.
        assert!(html.contains("No git history found"));
        assert!(html.contains("No decisions found"));
    }

    #[test]
    fn renders_hotspots_and_decisions_when_present() {
        let root = PathBuf::from("/repo");
        let overview = Overview {
            file_count: 1,
            other_file_count: 0,
            by_language: vec![],
            symbol_counts: vec![],
            total_lines: 1,
            import_edges: 0,
            call_edges: 0,
            unresolved_imports: 0,
            unresolved_calls: 0,
            most_depended_on: vec![],
        };
        let health = repowise_health::HealthReport {
            file_scores: vec![],
            findings: vec![],
            average_score: 10.0,
        };
        let hotspots = vec![Hotspot {
            file: root.join("hot.rs"),
            churn: 4,
            total_complexity: 10,
            bugfix_commits: 1,
            score: 40,
            decayed_score: 40.0,
            last_touch: None,
        }];
        let decisions = vec![DecisionRecord {
            id: "ADR-0001".to_string(),
            title: "Use sled".to_string(),
            source: DecisionSource::Adr {
                file: root.join("docs/adr/0001.md"),
            },
            status: Some("Superseded by ADR-0002".to_string()),
            superseded_by: Some("ADR-0002".to_string()),
            date: Some("2026-01-01".to_string()),
            body: String::new(),
            linked_files: vec![root.join("hot.rs")],
        }];

        let html = render(
            &root,
            &overview,
            &health,
            Some(&hotspots),
            &decisions,
            &HashSet::new(),
        );

        assert!(html.contains(">hot.rs<"));
        assert!(html.contains("ADR-0001"));
        assert!(html.contains("superseded by ADR-0002"));
    }

    #[test]
    fn links_file_paths_to_their_wiki_page_only_when_one_exists() {
        let root = PathBuf::from("/repo");
        let overview = Overview {
            file_count: 2,
            other_file_count: 0,
            by_language: vec![],
            symbol_counts: vec![],
            total_lines: 10,
            import_edges: 0,
            call_edges: 0,
            unresolved_imports: 0,
            unresolved_calls: 0,
            most_depended_on: vec![(root.join("has_wiki.rs"), 1), (root.join("no_wiki.rs"), 1)],
        };
        let health = repowise_health::HealthReport {
            file_scores: vec![],
            findings: vec![],
            average_score: 10.0,
        };
        let mut wiki_pages = HashSet::new();
        wiki_pages.insert(root.join("has_wiki.rs"));

        let html = render(&root, &overview, &health, None, &[], &wiki_pages);

        assert!(html.contains("<a href=\"../wiki/has_wiki.rs.md\">has_wiki.rs</a>"));
        // No wiki page for this one -- plain text, not a broken link.
        assert!(html.contains(">no_wiki.rs<"));
        assert!(!html.contains("../wiki/no_wiki.rs"));
    }
}
