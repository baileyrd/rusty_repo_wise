//! Builds a dependency graph (files, symbols, `Contains`/`Imports`/`Calls`
//! edges) out of a `RepoIndex`, and answers queries against it.
//!
//! Import and call resolution are heuristic, directory-layout-based
//! best-effort matching, not real compiler name resolution — see
//! `repowise-parser` for why that tradeoff is made.

mod modpath;

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use repowise_core::{Language, RepoIndex, Symbol};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAX_CALL_FANOUT: usize = 6;

#[derive(Debug, Clone)]
pub enum Node {
    File(PathBuf),
    Symbol(Symbol),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Contains,
    Imports,
    Calls,
}

pub struct RepoGraph {
    pub graph: DiGraph<Node, EdgeKind>,
    file_index: HashMap<PathBuf, NodeIndex>,
    symbol_index: HashMap<String, NodeIndex>,
    pub unresolved_imports: usize,
    pub unresolved_calls: usize,
}

impl RepoGraph {
    pub fn build(index: &RepoIndex) -> Self {
        let mut graph = DiGraph::new();
        let mut file_index = HashMap::new();
        let mut symbol_index = HashMap::new();
        let mut name_index: HashMap<String, Vec<String>> = HashMap::new();
        let mut rust_modules = HashMap::new();
        let mut python_modules = HashMap::new();
        // Shared between Java, Kotlin, and Scala: all three use the same
        // JVM package-path convention, and a mixed-language project can
        // reasonably import one from another.
        let mut jvm_modules = HashMap::new();
        let mut go_modules = HashMap::new();
        let mut csharp_modules = HashMap::new();

        for file in &index.files {
            let fnode = graph.add_node(Node::File(file.path.clone()));
            file_index.insert(file.path.clone(), fnode);
            for sym in &file.symbols {
                let snode = graph.add_node(Node::Symbol(sym.clone()));
                symbol_index.insert(sym.id.clone(), snode);
                graph.add_edge(fnode, snode, EdgeKind::Contains);
                name_index
                    .entry(sym.name.clone())
                    .or_default()
                    .push(sym.id.clone());
            }
            match file.language {
                Language::Rust => {
                    if let Some(mp) = modpath::rust_module_path(&file.path) {
                        rust_modules.insert(mp, file.path.clone());
                    }
                }
                Language::Python => {
                    if let Some(mp) = modpath::python_module_path(&file.path, &index.root) {
                        python_modules.insert(mp, file.path.clone());
                    }
                }
                Language::Java | Language::Kotlin | Language::Scala => {
                    if let Some(mp) = modpath::jvm_module_path(&file.path, &index.root) {
                        jvm_modules.insert(mp, file.path.clone());
                    }
                }
                Language::Go => {
                    if let Some(mp) = modpath::go_module_path(&file.path) {
                        go_modules.insert(mp, file.path.clone());
                    }
                }
                Language::CSharp => {
                    if let Some(mp) = modpath::csharp_namespace_path(&file.path, &index.root) {
                        csharp_modules.insert(mp, file.path.clone());
                    }
                }
                // TypeScript/JavaScript/C++ relative (quote-form)
                // imports are resolved directly at parse time (see
                // `resolve_relative_import`/`resolve_include` in
                // `repowise-parser`), so there's no module-path index to
                // build here, unlike Rust/Python/Java/Kotlin/Go/C#/Scala's
                // dotted/`::`/`/`-separated paths.
                Language::TypeScript | Language::JavaScript | Language::Cpp | Language::Other => {}
            }
        }

        let mut unresolved_imports = 0usize;
        let mut unresolved_calls = 0usize;
        let no_modules = HashMap::new();

        for file in &index.files {
            let from = file_index[&file.path];
            let (sep, map): (&str, &HashMap<String, PathBuf>) = match file.language {
                Language::Rust => ("::", &rust_modules),
                Language::Python => (".", &python_modules),
                Language::Java | Language::Kotlin | Language::Scala => (".", &jvm_modules),
                Language::Go => ("/", &go_modules),
                Language::CSharp => (".", &csharp_modules),
                Language::TypeScript | Language::JavaScript | Language::Cpp => ("", &no_modules),
                Language::Other => continue,
            };
            for imp in &file.imports {
                let target = match &imp.resolved_file {
                    Some(t) => Some(t),
                    None => modpath::resolve_import(&imp.path, sep, map),
                };
                match target {
                    Some(target) if target != &file.path => {
                        if let Some(&to) = file_index.get(target) {
                            graph.add_edge(from, to, EdgeKind::Imports);
                        } else {
                            unresolved_imports += 1;
                        }
                    }
                    Some(_) => {} // self-import (e.g. re-export within same file); ignore
                    None => unresolved_imports += 1,
                }
            }
        }

        for file in &index.files {
            for call in &file.calls {
                let from = match &call.caller {
                    Some(cid) => match symbol_index.get(cid) {
                        Some(&idx) => idx,
                        None => continue,
                    },
                    None => file_index[&file.path],
                };
                let caller_file = call
                    .caller
                    .as_ref()
                    .and_then(|cid| symbol_index.get(cid))
                    .and_then(|&idx| graph.node_weight(idx))
                    .and_then(|n| match n {
                        Node::Symbol(s) => Some(s.file.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| file.path.clone());

                let Some(target_ids) = name_index.get(&call.callee_name) else {
                    unresolved_calls += 1;
                    continue;
                };

                let mut candidates: Vec<(NodeIndex, bool)> = Vec::new();
                for tid in target_ids {
                    if let Some(&idx) = symbol_index.get(tid) {
                        let same_file = matches!(
                            graph.node_weight(idx),
                            Some(Node::Symbol(s)) if s.file == caller_file
                        );
                        candidates.push((idx, same_file));
                    }
                }
                let same_file: Vec<NodeIndex> = candidates
                    .iter()
                    .filter(|(_, same)| *same)
                    .map(|(idx, _)| *idx)
                    .collect();
                let chosen: Vec<NodeIndex> = if !same_file.is_empty() {
                    same_file
                } else {
                    candidates
                        .iter()
                        .take(MAX_CALL_FANOUT)
                        .map(|(idx, _)| *idx)
                        .collect()
                };

                if chosen.is_empty() {
                    unresolved_calls += 1;
                }
                for to in chosen {
                    if to != from {
                        graph.add_edge(from, to, EdgeKind::Calls);
                    }
                }
            }
        }

        RepoGraph {
            graph,
            file_index,
            symbol_index,
            unresolved_imports,
            unresolved_calls,
        }
    }

    pub fn file_node(&self, path: &Path) -> Option<NodeIndex> {
        self.file_index.get(path).copied()
    }

    pub fn symbol_node(&self, id: &str) -> Option<NodeIndex> {
        self.symbol_index.get(id).copied()
    }

    /// Number of resolved call sites targeting this symbol. `0` is a
    /// best-effort "possibly unused" signal, not a guarantee: it misses
    /// calls this heuristic couldn't resolve (see `unresolved_calls`),
    /// trait-dispatched calls, and external/reflective callers.
    pub fn call_in_degree(&self, symbol_id: &str) -> usize {
        let Some(idx) = self.symbol_node(symbol_id) else {
            return 0;
        };
        self.graph
            .edges_directed(idx, Direction::Incoming)
            .filter(|e| *e.weight() == EdgeKind::Calls)
            .count()
    }

    /// Case-insensitive substring search over symbol names.
    pub fn search(&self, query: &str) -> Vec<&Symbol> {
        let q = query.to_lowercase();
        self.graph
            .node_weights()
            .filter_map(|n| match n {
                Node::Symbol(s) if s.name.to_lowercase().contains(&q) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// Files that `file` imports (best-effort resolved, deduplicated).
    pub fn dependencies_of(&self, file: &Path) -> Vec<PathBuf> {
        let Some(idx) = self.file_node(file) else {
            return Vec::new();
        };
        let mut out: Vec<PathBuf> = self
            .graph
            .edges_directed(idx, Direction::Outgoing)
            .filter(|e| *e.weight() == EdgeKind::Imports)
            .filter_map(|e| match &self.graph[e.target()] {
                Node::File(p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Files that import `file` (best-effort resolved, deduplicated).
    pub fn dependents_of(&self, file: &Path) -> Vec<PathBuf> {
        let Some(idx) = self.file_node(file) else {
            return Vec::new();
        };
        let mut out: Vec<PathBuf> = self
            .graph
            .edges_directed(idx, Direction::Incoming)
            .filter(|e| *e.weight() == EdgeKind::Imports)
            .filter_map(|e| match &self.graph[e.source()] {
                Node::File(p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn overview(&self, index: &RepoIndex) -> Overview {
        let mut by_language: HashMap<&'static str, usize> = HashMap::new();
        let mut symbol_counts: HashMap<&'static str, usize> = HashMap::new();
        let mut total_lines = 0usize;

        for file in &index.files {
            *by_language.entry(file.language.label()).or_default() += 1;
            total_lines += file.lines;
            for sym in &file.symbols {
                *symbol_counts.entry(sym.kind.label()).or_default() += 1;
            }
        }

        let mut import_edges = 0usize;
        let mut call_edges = 0usize;
        for e in self.graph.edge_weights() {
            match e {
                EdgeKind::Imports => import_edges += 1,
                EdgeKind::Calls => call_edges += 1,
                EdgeKind::Contains => {}
            }
        }

        let mut most_depended_on: Vec<(PathBuf, usize)> = self
            .file_index
            .keys()
            .map(|path| (path.clone(), self.dependents_of(path).len()))
            .filter(|(_, c)| *c > 0)
            .collect();
        most_depended_on.sort_by_key(|b| std::cmp::Reverse(b.1));
        most_depended_on.truncate(10);

        let mut by_language: Vec<(String, usize)> = by_language
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        by_language.sort_by_key(|b| std::cmp::Reverse(b.1));

        let mut symbol_counts: Vec<(String, usize)> = symbol_counts
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        symbol_counts.sort_by_key(|b| std::cmp::Reverse(b.1));

        Overview {
            file_count: index.files.len(),
            other_file_count: index.other_files,
            by_language,
            symbol_counts,
            total_lines,
            import_edges,
            call_edges,
            unresolved_imports: self.unresolved_imports,
            unresolved_calls: self.unresolved_calls,
            most_depended_on,
        }
    }
}

pub struct Overview {
    pub file_count: usize,
    pub other_file_count: usize,
    pub by_language: Vec<(String, usize)>,
    pub symbol_counts: Vec<(String, usize)>,
    pub total_lines: usize,
    pub import_edges: usize,
    pub call_edges: usize,
    pub unresolved_imports: usize,
    pub unresolved_calls: usize,
    /// (file, number of files that import it), most depended-on first.
    pub most_depended_on: Vec<(PathBuf, usize)>,
}
