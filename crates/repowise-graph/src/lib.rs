//! Builds a dependency graph (files, symbols, `Contains`/`Imports`/`Calls`
//! edges) out of a `RepoIndex`, and answers queries against it.
//!
//! Import and call resolution are heuristic, directory-layout-based
//! best-effort matching, not real compiler name resolution — see
//! `repowise-parser` for why that tradeoff is made.

mod community;
mod cross_repo;
mod modpath;
mod search;

pub mod json_graph;

pub use community::detect_communities;
pub use search::{classify, parse_symbol_kind, path_matches, FileKind, SearchMode};

pub use cross_repo::{
    cross_repo_import_edges, detect_repo_cycles, module_map, rust_module_map, CrossRepoImportEdge,
    MODULE_MAP_LANGUAGES,
};

use petgraph::algo::kosaraju_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use repowise_core::{Language, RepoIndex, Symbol};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const MAX_CALL_FANOUT: usize = 6;

/// The part of an import path after its final `.`/`::`/`/`/`\` separator
/// (whichever appears last), lowercased. Language-naive on purpose —
/// used only as a coarse "might this unresolved import have meant this
/// file" signal, not for actual resolution.
fn last_path_segment(path: &str) -> String {
    path.rsplit(['.', ':', '/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase()
}

#[derive(Debug, Clone)]
pub enum Node {
    File(PathBuf),
    /// Boxed since `Symbol` (now carrying several `Vec<...>` health-marker
    /// fields) is otherwise much larger than the `File` variant --
    /// `clippy::large_enum_variant`.
    Symbol(Box<Symbol>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Contains,
    Imports,
    Calls,
}

#[derive(Clone)]
pub struct RepoGraph {
    pub graph: DiGraph<Node, EdgeKind>,
    file_index: HashMap<PathBuf, NodeIndex>,
    symbol_index: HashMap<String, NodeIndex>,
    pub unresolved_imports: usize,
    pub unresolved_calls: usize,
    /// Distinct last path segments (the part after the final `.`/`::`/
    /// `/`/`\`) among imports that failed to resolve to any indexed file.
    /// Used by `repowise-health`'s dead-code confidence tiering: an
    /// unresolved import whose last segment matches a file's stem is a
    /// plausible (if coarse, language-naive) sign that something meant
    /// to import that file, but this port's directory-layout heuristics
    /// couldn't confirm it — so a "zero callers" reading for a symbol in
    /// that file is less trustworthy than it looks.
    pub unresolved_import_stems: HashSet<String>,
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
        let mut php_modules = HashMap::new();

        for file in &index.files {
            let fnode = graph.add_node(Node::File(file.path.clone()));
            file_index.insert(file.path.clone(), fnode);
            for sym in &file.symbols {
                let snode = graph.add_node(Node::Symbol(Box::new(sym.clone())));
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
                // PHP has two import forms: `require`/`include` (already
                // resolved directly against the filesystem at parse time,
                // same as C/C++/Ruby, bypassing this map entirely) and
                // `use Namespace\Class;` (resolved via this PSR-4-style
                // folder-mirrors-namespace map, same convention as C#).
                Language::Php => {
                    if let Some(mp) = modpath::php_namespace_path(&file.path, &index.root) {
                        php_modules.insert(mp, file.path.clone());
                    }
                }
                // TypeScript/JavaScript/C/C++/Ruby relative (quote-form/
                // `require_relative`) imports are resolved directly at
                // parse time (see `resolve_relative_import`/
                // `resolve_include`/`resolve_require_relative` in
                // `repowise-parser`), so there's no module-path index to
                // build here, unlike Rust/Python/Java/Kotlin/Go/C#/Scala's
                // dotted/`::`/`/`-separated paths. Swift's module-level
                // `import` has no per-file mapping at all (no build graph
                // to resolve against), so its imports are always left
                // unresolved by design, same "no index needed" bucket.
                // Dart's relative `import 'local.dart'` is likewise
                // resolved directly at parse time (see
                // `resolve_relative_import` in `repowise-parser`);
                // `package:x/y.dart` imports have no pub-package registry
                // here to resolve against and are always left unresolved,
                // the same "no index needed" bucket as Swift's imports.
                // Shell's `source`/`.` (including the `$SCRIPT_DIR`
                // self-relative idiom) is likewise resolved directly at
                // parse time (see `resolve_relative` in
                // `repowise-parser`); any other expansion has no static
                // value to resolve, joining the same "no index needed"
                // bucket.
                // The 9 "Structural tier" languages (issue #70) never
                // reach this match at all in practice -- `parse_file`
                // gives them an empty `imports: Vec::new()`, so there's
                // nothing to resolve -- but they still need a home in
                // this exhaustive match; same "no index needed" bucket.
                // Dockerfile (#318) joins them for the same reason: its
                // `FileRecord` is the same zero-symbol, zero-import
                // shape -- its actual content lives in the separate
                // `DockerStage`/`DockerCopyFromEdge` model instead.
                // The Lightweight tier (#69) has real imports, unlike
                // the languages above, but has no module-path map to
                // build one from either -- its `ImportRef`s stay
                // unresolved by design (see `repowise_parser::
                // lightweight`'s module doc), so it needs no branch
                // here beyond a home in this exhaustive match.
                // Luau's only import mechanism is `require(...)` (see
                // `repowise_parser::luau`'s module doc): a Roblox
                // instance-tree path (`script.Parent.Foo`) has no
                // filesystem mapping at all, and a plain string path has
                // no fixed extension/directory convention the way JS's
                // `./` imports do -- so, like Swift's/Dart's package
                // imports, there's no module-path index to build and
                // every `require` stays unresolved by design, joining the
                // same "no index needed" bucket.
                Language::TypeScript
                | Language::JavaScript
                | Language::C
                | Language::Cpp
                | Language::Ruby
                | Language::Swift
                | Language::Dart
                | Language::Shell
                | Language::Luau
                | Language::ObjectiveC
                | Language::R
                | Language::Zig
                | Language::Julia
                | Language::Elm
                | Language::OCaml
                | Language::Crystal
                | Language::Nim
                | Language::D
                | Language::Dockerfile
                | Language::Sql
                | Language::Proto
                | Language::GraphQl
                | Language::Terraform
                | Language::Elixir
                | Language::Clojure
                | Language::Haskell
                | Language::Lean
                | Language::Erlang
                | Language::FSharp
                | Language::Other => {}
            }
        }

        let mut unresolved_imports = 0usize;
        let mut unresolved_calls = 0usize;
        let mut unresolved_import_stems: HashSet<String> = HashSet::new();
        let no_modules = HashMap::new();

        for file in &index.files {
            let from = file_index[&file.path];
            let (sep, map): (&str, &HashMap<String, PathBuf>) = match file.language {
                Language::Rust => ("::", &rust_modules),
                Language::Python => (".", &python_modules),
                Language::Java | Language::Kotlin | Language::Scala => (".", &jvm_modules),
                Language::Go => ("/", &go_modules),
                Language::CSharp => (".", &csharp_modules),
                Language::Php => ("\\", &php_modules),
                Language::TypeScript
                | Language::JavaScript
                | Language::C
                | Language::Cpp
                | Language::Ruby
                | Language::Swift
                | Language::Dart
                | Language::Shell
                | Language::Luau
                | Language::ObjectiveC
                | Language::R
                | Language::Zig
                | Language::Julia
                | Language::Elm
                | Language::OCaml
                | Language::Crystal
                | Language::Nim
                | Language::D
                | Language::Dockerfile
                | Language::Sql
                | Language::Proto
                | Language::GraphQl
                | Language::Terraform
                | Language::Elixir
                | Language::Clojure
                | Language::Haskell
                | Language::Lean
                | Language::Erlang
                | Language::FSharp => ("", &no_modules),
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
                            unresolved_import_stems.insert(last_path_segment(&imp.path));
                        }
                    }
                    Some(_) => {} // self-import (e.g. re-export within same file); ignore
                    None => {
                        unresolved_imports += 1;
                        unresolved_import_stems.insert(last_path_segment(&imp.path));
                    }
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
            unresolved_import_stems,
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
                Node::Symbol(s) if s.name.to_lowercase().contains(&q) => Some(s.as_ref()),
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

    /// Groups of files whose (best-effort resolved) imports form a
    /// cycle: A imports B imports ... imports A. A file that imports
    /// itself counts too (a self-loop, reported as a one-file group).
    ///
    /// This is the single-repo counterpart to
    /// `cross_repo::detect_repo_cycles` -- same technique (Kosaraju
    /// strongly-connected components over a directed graph), applied to
    /// this repo's own file-level `Imports` edges rather than
    /// workspace-level repo-to-repo edges. Nothing computed this before:
    /// `dependencies_of`/`dependents_of` answer "what does this one file
    /// touch", not "is there a cycle anywhere", and a cycle is exactly
    /// the shape a `--mode symbol`/path search can't see because no
    /// single file's own dependency list looks wrong in isolation.
    ///
    /// Groups of 1 are only ever a genuine self-import; an acyclic file
    /// is simply absent, not returned as a trivial group of itself --
    /// SCC computation always produces one, so it's filtered here rather
    /// than pushed onto every caller.
    pub fn file_import_cycles(&self) -> Vec<Vec<PathBuf>> {
        let mut sub = DiGraph::<PathBuf, ()>::new();
        let mut sub_index: HashMap<PathBuf, NodeIndex> = HashMap::new();
        let mut self_loops: HashSet<PathBuf> = HashSet::new();

        for edge in self.graph.edge_references() {
            if *edge.weight() != EdgeKind::Imports {
                continue;
            }
            let (Node::File(from), Node::File(to)) =
                (&self.graph[edge.source()], &self.graph[edge.target()])
            else {
                continue;
            };
            if from == to {
                self_loops.insert(from.clone());
            }
            let a = *sub_index
                .entry(from.clone())
                .or_insert_with(|| sub.add_node(from.clone()));
            let b = *sub_index
                .entry(to.clone())
                .or_insert_with(|| sub.add_node(to.clone()));
            sub.add_edge(a, b, ());
        }

        kosaraju_scc(&sub)
            .into_iter()
            .filter_map(|scc| {
                if scc.len() > 1 {
                    Some(scc.into_iter().map(|idx| sub[idx].clone()).collect())
                } else {
                    let file = &sub[scc[0]];
                    self_loops.contains(file).then(|| vec![file.clone()])
                }
            })
            .collect()
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
