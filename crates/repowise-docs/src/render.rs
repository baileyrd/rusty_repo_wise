use repowise_core::{FileRecord, Symbol};
use repowise_graph::RepoGraph;
use repowise_health::Finding;
use std::path::Path;

/// Render one file's wiki page: a symbol list, resolved dependencies/
/// dependents, and health findings — all sourced from data
/// `repowise-parser`/`repowise-graph`/`repowise-health` already computed.
/// `content_hash` is embedded as the first line so a later run can tell
/// whether this file's own source has changed since.
pub fn render_page(
    file: &FileRecord,
    content_hash: u64,
    graph: &RepoGraph,
    root: &Path,
    findings: &[&Finding],
) -> String {
    let rel = display_rel(&file.path, root);
    let mut out = String::new();

    out.push_str(&format!("<!-- content-hash: {content_hash} -->\n"));
    out.push_str(&format!("# {rel}\n\n"));
    out.push_str(&format!("**Language:** {}  \n", file.language.label()));
    out.push_str(&format!("**Lines:** {}\n\n", file.lines));

    out.push_str("## Symbols\n\n");
    if file.symbols.is_empty() {
        out.push_str("_No symbols indexed._\n\n");
    } else {
        let mut symbols: Vec<&Symbol> = file.symbols.iter().collect();
        symbols.sort_by_key(|s| s.start_line);
        for sym in symbols {
            match &sym.parent {
                Some(parent) => out.push_str(&format!(
                    "- `{}` **{}** (in `{parent}`) — line {}\n",
                    sym.kind.label(),
                    sym.name,
                    sym.start_line
                )),
                None => out.push_str(&format!(
                    "- `{}` **{}** — line {}\n",
                    sym.kind.label(),
                    sym.name,
                    sym.start_line
                )),
            }
        }
        out.push('\n');
    }

    out.push_str("## Dependencies\n\n");
    let deps = graph.dependencies_of(&file.path);
    if deps.is_empty() {
        out.push_str("- Depends on: _none resolved_\n");
    } else {
        out.push_str("- Depends on:\n");
        for d in &deps {
            out.push_str(&format!("  - `{}`\n", display_rel(d, root)));
        }
    }
    let dependents = graph.dependents_of(&file.path);
    if dependents.is_empty() {
        out.push_str("- Depended on by: _none resolved_\n");
    } else {
        out.push_str("- Depended on by:\n");
        for d in &dependents {
            out.push_str(&format!("  - `{}`\n", display_rel(d, root)));
        }
    }
    out.push('\n');

    out.push_str("## Health\n\n");
    if findings.is_empty() {
        out.push_str("_No findings._\n");
    } else {
        for f in findings {
            let symbol_prefix = f
                .symbol
                .as_deref()
                .map(|s| format!("`{s}` — "))
                .unwrap_or_default();
            out.push_str(&format!(
                "- **{}**: {}{}\n",
                f.kind.label(),
                symbol_prefix,
                f.detail
            ));
        }
    }

    out
}

fn display_rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{CallRef, Language, RepoIndex};
    use repowise_health::{Finding, FindingKind};
    use std::path::PathBuf;

    #[test]
    fn renders_symbols_dependencies_and_health_findings() {
        let root = PathBuf::from("/repo");
        let a_path = root.join("a.rs");
        let b_path = root.join("b.rs");

        let symbol = Symbol {
            id: Symbol::make_id(&a_path, "widget", 3),
            name: "widget".to_string(),
            kind: repowise_core::SymbolKind::Function,
            file: a_path.clone(),
            start_line: 3,
            end_line: 3,
            parent: None,
            complexity: 12,
            max_nesting_depth: 0,
            bumpy_road_bumps: 0,
            param_count: 1,
            body_hash: None,
        };
        let call = CallRef {
            caller: None,
            callee_name: "helper".to_string(),
            line: 4,
        };
        let a_file = FileRecord {
            path: a_path.clone(),
            language: Language::Other,
            lines: 10,
            symbols: vec![symbol],
            imports: vec![],
            calls: vec![call],
            field_accesses: vec![],
        };
        let b_file = FileRecord {
            path: b_path.clone(),
            language: Language::Other,
            lines: 5,
            symbols: vec![],
            imports: vec![],
            calls: vec![],
            field_accesses: vec![],
        };
        let index = RepoIndex {
            root: root.clone(),
            files: vec![a_file.clone(), b_file],
            other_files: 0,
        };
        let graph = RepoGraph::build(&index);

        let finding = Finding {
            file: a_path.clone(),
            symbol: Some("widget".to_string()),
            line: Some(3),
            kind: FindingKind::HighComplexity,
            detail: "cyclomatic complexity 12 (> 10)".to_string(),
        };
        let findings = vec![&finding];

        let page = render_page(&a_file, 42, &graph, &root, &findings);

        assert!(page.starts_with("<!-- content-hash: 42 -->\n"));
        assert!(page.contains("# a.rs"));
        assert!(page.contains("`function` **widget** — line 3"));
        assert!(page.contains("## Health"));
        assert!(page.contains("**high-complexity**: `widget` — cyclomatic complexity 12 (> 10)"));
    }
}
