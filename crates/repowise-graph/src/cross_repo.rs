//! Cross-repo import resolution: an unresolved import in one workspace
//! repo, resolved against another workspace repo's module-path map.
//!
//! Covers every language this port already resolves single-repo via a
//! name -> file module map: Rust, Python, Java/Kotlin/Scala, Go, C#,
//! and PHP's `use Namespace\Class;` form (see `MODULE_MAP_LANGUAGES`
//! and `RepoGraph::build`'s own per-language `(separator, map)` table,
//! which this reuses rather than re-deriving). TypeScript/JavaScript/C/
//! C++/Ruby/Swift/Dart/Shell resolve imports directly against the
//! filesystem at parse time instead of through a name -> file map --  a
//! different resolution mechanism entirely (it would mean walking every
//! sibling repo's filesystem looking for a relative-path match, not
//! matching a dotted/`::`/`/`-separated name), so they have no
//! cross-repo equivalent here; a future slice. Luau's `require(...)`
//! joins that same "no index needed" bucket, but for a different reason:
//! it has no module-path map to resolve against *at all*, single-repo or
//! cross-repo (see `repowise_parser::luau`'s module doc). The Structural
//! and Lightweight tiers carry no module-path concept at all (no
//! grammar, or regex-only unresolved imports respectively) and were
//! never candidates for this pass.
//!
//! This is a separate pass from `RepoGraph::build`, which only ever
//! sees one repo's `RepoIndex` at a time. Callers (`repowise-workspace`)
//! supply every configured repo's `RepoIndex` up front.

use crate::modpath;
use petgraph::algo::kosaraju_scc;
use petgraph::graph::DiGraph;
use repowise_core::{Language, RepoIndex};
use std::collections::HashMap;
use std::path::PathBuf;

/// Every language this cross-repo pass resolves, in the same order
/// `RepoGraph::build` lists them -- see this module's own doc comment
/// for why the rest are out of scope here.
pub const MODULE_MAP_LANGUAGES: &[Language] = &[
    Language::Rust,
    Language::Python,
    Language::Java,
    Language::Kotlin,
    Language::Scala,
    Language::Go,
    Language::CSharp,
    Language::Php,
];

/// This language's import-path separator, matching `RepoGraph::build`'s
/// own table exactly. Empty for a language `module_map` never returns
/// entries for.
fn separator(language: Language) -> &'static str {
    match language {
        Language::Rust => "::",
        Language::Python
        | Language::Java
        | Language::Kotlin
        | Language::Scala
        | Language::CSharp => ".",
        Language::Go => "/",
        Language::Php => "\\",
        _ => "",
    }
}

/// name -> defining file for one `language` in one repo's `RepoIndex`.
/// A freestanding duplicate of the maps `RepoGraph::build` computes
/// internally for its own single-repo resolution (same `modpath::*`
/// function per language) -- kept as a pure function so it's usable
/// per-repo before any single-repo `RepoGraph` exists, without building
/// a full `RepoGraph` per other repo just to read one field back out.
/// Empty for any language outside `MODULE_MAP_LANGUAGES`.
pub fn module_map(index: &RepoIndex, language: Language) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    for file in &index.files {
        if file.language != language {
            continue;
        }
        let mp = match language {
            Language::Rust => modpath::rust_module_path(&file.path),
            Language::Python => modpath::python_module_path(&file.path, &index.root),
            Language::Java | Language::Kotlin | Language::Scala => {
                modpath::jvm_module_path(&file.path, &index.root)
            }
            Language::Go => modpath::go_module_path(&file.path),
            Language::CSharp => modpath::csharp_namespace_path(&file.path, &index.root),
            Language::Php => modpath::php_namespace_path(&file.path, &index.root),
            _ => None,
        };
        if let Some(mp) = mp {
            map.insert(mp, file.path.clone());
        }
    }
    map
}

/// Rust module path -> defining file, for one repo's `RepoIndex`.
/// Retained as a thin wrapper over `module_map` for existing callers
/// that only ever wanted Rust (e.g. tests written before this pass
/// covered other languages) -- new code should call `module_map`
/// directly with the language it needs.
pub fn rust_module_map(index: &RepoIndex) -> HashMap<String, PathBuf> {
    module_map(index, Language::Rust)
}

/// One `use` import in `from_repo`/`from_file` resolved to a specific
/// file in a *different* workspace repo. First match across other repos
/// wins (in the order given to `cross_repo_import_edges`) -- cross-repo
/// name collisions are rare and out of scope to disambiguate further.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossRepoImportEdge {
    pub from_repo: String,
    pub from_file: PathBuf,
    pub line: usize,
    pub to_repo: String,
    pub to_file: PathBuf,
    pub import_path: String,
}

/// Cross-repo import resolution over every repo in `repos`, for every
/// language in `MODULE_MAP_LANGUAGES`.
///
/// An import counts as a cross-repo candidate only if it's unresolved
/// BOTH at parse time (`resolved_file: None`) AND against its own
/// repo's own-language module map -- the same two-part "did this end up
/// unresolved" test `RepoGraph::build` applies for single-repo
/// resolution, re-derived here so an import already fully explained by
/// a sibling crate/package within the SAME repo (this port's own
/// multi-crate layout, for example) is never mistaken for a cross-repo
/// edge, even if another repo happens to define a module at the same
/// dotted path. Each language's maps are built once per repo up front
/// (`per_language_maps`, keyed by `(repo_index, language)`) rather than
/// once per file, since multiple files in one repo share the same
/// language's map.
pub fn cross_repo_import_edges(repos: &[(String, RepoIndex)]) -> Vec<CrossRepoImportEdge> {
    let per_language_maps: Vec<HashMap<Language, HashMap<String, PathBuf>>> = repos
        .iter()
        .map(|(_, index)| {
            MODULE_MAP_LANGUAGES
                .iter()
                .map(|&lang| (lang, module_map(index, lang)))
                .collect()
        })
        .collect();

    let mut edges = Vec::new();
    for (i, (from_repo, index)) in repos.iter().enumerate() {
        for file in &index.files {
            if !MODULE_MAP_LANGUAGES.contains(&file.language) {
                continue;
            }
            let sep = separator(file.language);
            let own_map = &per_language_maps[i][&file.language];
            for imp in &file.imports {
                if imp.resolved_file.is_some() {
                    continue;
                }
                if modpath::resolve_import(&imp.path, sep, own_map).is_some() {
                    continue; // resolves within its own repo -- not a cross-repo candidate
                }
                for (j, (other_repo, _)) in repos.iter().enumerate() {
                    if other_repo == from_repo {
                        continue;
                    }
                    let other_map = &per_language_maps[j][&file.language];
                    if let Some(target) = modpath::resolve_import(&imp.path, sep, other_map) {
                        edges.push(CrossRepoImportEdge {
                            from_repo: from_repo.clone(),
                            from_file: file.path.clone(),
                            line: imp.line,
                            to_repo: other_repo.clone(),
                            to_file: target.clone(),
                            import_path: imp.path.clone(),
                        });
                        break;
                    }
                }
            }
        }
    }
    edges
}

/// Detects cycles among repo-level edges (e.g. repo A imports repo B
/// imports repo A). Generic over repo-name string pairs rather than
/// `CrossRepoImportEdge`s -- operates on an aggregated repo-to-repo
/// edge list, not individual import sites. Each returned group is the
/// set of repo names involved in one cycle (order not meaningful); a
/// repo with no cycle partner is omitted, not returned as a singleton.
pub fn detect_repo_cycles(repo_edges: &[(String, String)]) -> Vec<Vec<String>> {
    let mut graph = DiGraph::<String, ()>::new();
    let mut index_of = HashMap::new();
    let mut self_loops = std::collections::HashSet::new();
    for (from, to) in repo_edges {
        if from == to {
            self_loops.insert(from.clone());
        }
        let a = *index_of
            .entry(from.clone())
            .or_insert_with(|| graph.add_node(from.clone()));
        let b = *index_of
            .entry(to.clone())
            .or_insert_with(|| graph.add_node(to.clone()));
        graph.add_edge(a, b, ());
    }

    kosaraju_scc(&graph)
        .into_iter()
        .filter_map(|scc| {
            if scc.len() > 1 {
                Some(scc.into_iter().map(|idx| graph[idx].clone()).collect())
            } else {
                let name = &graph[scc[0]];
                self_loops.contains(name).then(|| vec![name.clone()])
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_repo_cycles_finds_a_two_repo_cycle() {
        let edges = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "a".to_string()),
        ];
        let cycles = detect_repo_cycles(&edges);
        assert_eq!(cycles.len(), 1);
        let mut cycle = cycles[0].clone();
        cycle.sort();
        assert_eq!(cycle, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn detect_repo_cycles_reports_none_for_a_dag() {
        let edges = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "c".to_string()),
        ];
        assert!(detect_repo_cycles(&edges).is_empty());
    }

    #[test]
    fn detect_repo_cycles_ignores_isolated_repos_with_no_edges() {
        assert!(detect_repo_cycles(&[]).is_empty());
    }
}
