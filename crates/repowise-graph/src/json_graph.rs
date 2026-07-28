//! [JSON Graph Format](https://jsongraphformat.info/) serialization of
//! the dependency graph — the architecture-model half of issue #244.
//!
//! JGF was chosen over Graphviz DOT and Mermaid because it's the only
//! one of the three that carries per-node metadata losslessly: this
//! graph's nodes know their language, line count, symbol kind,
//! complexity and nesting depth, and DOT/Mermaid would have to discard
//! all of it to render a picture.
//!
//! # Honesty about what isn't here
//!
//! This port resolves imports and calls with directory-layout
//! heuristics, **not** compiler-grade name resolution, and deliberately
//! leaves ambiguous or external references unresolved rather than
//! guessing at them. Those references therefore have no edge in this
//! export, because they have no target node to point at.
//!
//! Silently emitting a graph that merely *looks* complete would be the
//! worst outcome — a consumer would read missing edges as "nothing
//! depends on this". So the graph-level metadata declares the unresolved
//! counts and names the import stems that failed to resolve. The export
//! is partial by construction, and says so.

use crate::{EdgeKind, Node, RepoGraph};
use repowise_core::RepoIndex;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct JsonGraphDocument {
    pub graph: JsonGraph,
}

#[derive(Debug, Serialize)]
pub struct JsonGraph {
    pub id: String,
    #[serde(rename = "type")]
    pub graph_type: String,
    pub label: String,
    pub directed: bool,
    pub metadata: GraphMetadata,
    /// Keyed by node id, per JGF v2.
    pub nodes: BTreeMap<String, JsonNode>,
    pub edges: Vec<JsonEdge>,
}

#[derive(Debug, Serialize)]
pub struct GraphMetadata {
    pub generator: String,
    pub file_count: usize,
    pub symbol_count: usize,
    pub unresolved: UnresolvedMetadata,
}

/// What this export could not represent, and why. See the module doc.
#[derive(Debug, Serialize)]
pub struct UnresolvedMetadata {
    pub imports: usize,
    pub calls: usize,
    /// Last path segment of each import that failed to resolve, sorted.
    /// Deduplicated repo-wide, so these are names rather than sites --
    /// which is why they appear here rather than as edges.
    pub import_stems: Vec<String>,
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct JsonNode {
    pub label: String,
    pub metadata: NodeMetadata,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum NodeMetadata {
    File {
        kind: &'static str,
        path: String,
        language: String,
        lines: usize,
    },
    Symbol {
        kind: &'static str,
        symbol_kind: String,
        file: String,
        start_line: usize,
        end_line: usize,
        complexity: usize,
        max_nesting_depth: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
    },
}

#[derive(Debug, Serialize)]
pub struct JsonEdge {
    pub source: String,
    pub target: String,
    pub relation: &'static str,
    pub directed: bool,
}

/// Repo-relative path with forward slashes, so an export is portable
/// between platforms rather than carrying the producing machine's
/// separators.
fn rel_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn file_id(path: &Path, root: &Path) -> String {
    format!("file:{}", rel_path(path, root))
}

/// Symbol node id, built from the symbol's fields rather than reusing
/// `Symbol::id` verbatim.
///
/// `Symbol::make_id` embeds the file's **absolute** path, which is fine
/// in-process but would bake the producing machine's directory layout
/// into every exported id. This mirrors that id's shape with a
/// repo-relative path instead, so an export is portable. Both the node
/// map and the edge list go through this one function, so the two cannot
/// disagree.
fn symbol_id(sym: &repowise_core::Symbol, root: &Path) -> String {
    format!(
        "symbol:{}::{}@{}",
        rel_path(&sym.file, root),
        sym.name,
        sym.start_line
    )
}

fn relation_of(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Contains => "contains",
        EdgeKind::Imports => "imports",
        EdgeKind::Calls => "calls",
    }
}

impl RepoGraph {
    /// Serialize this graph as a JGF document.
    ///
    /// Node and edge ordering is deterministic (nodes keyed in a
    /// `BTreeMap`, edges sorted) so two exports of an unchanged repo are
    /// byte-identical -- which is what makes the output diffable and
    /// snapshot-testable.
    pub fn to_json_graph(&self, index: &RepoIndex) -> JsonGraphDocument {
        let root = &index.root;
        let mut nodes = BTreeMap::new();
        let mut symbol_count = 0usize;

        for idx in self.graph.node_indices() {
            match &self.graph[idx] {
                Node::File(path) => {
                    let rel = rel_path(path, root);
                    let language = index
                        .files
                        .iter()
                        .find(|f| &f.path == path)
                        .map(|f| f.language.label().to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    let lines = index
                        .files
                        .iter()
                        .find(|f| &f.path == path)
                        .map(|f| f.lines)
                        .unwrap_or(0);
                    nodes.insert(
                        file_id(path, root),
                        JsonNode {
                            label: rel.clone(),
                            metadata: NodeMetadata::File {
                                kind: "file",
                                path: rel,
                                language,
                                lines,
                            },
                        },
                    );
                }
                Node::Symbol(sym) => {
                    symbol_count += 1;
                    nodes.insert(
                        symbol_id(sym, root),
                        JsonNode {
                            label: sym.name.clone(),
                            metadata: NodeMetadata::Symbol {
                                kind: "symbol",
                                symbol_kind: sym.kind.label().to_string(),
                                file: rel_path(&sym.file, root),
                                start_line: sym.start_line,
                                end_line: sym.end_line,
                                complexity: sym.complexity,
                                max_nesting_depth: sym.max_nesting_depth,
                                parent: sym.parent.clone(),
                            },
                        },
                    );
                }
            }
        }

        let node_id = |idx: petgraph::graph::NodeIndex| -> String {
            match &self.graph[idx] {
                Node::File(p) => file_id(p, root),
                Node::Symbol(s) => symbol_id(s, root),
            }
        };

        let mut edges: Vec<JsonEdge> = self
            .graph
            .edge_indices()
            .filter_map(|e| {
                let (from, to) = self.graph.edge_endpoints(e)?;
                Some(JsonEdge {
                    source: node_id(from),
                    target: node_id(to),
                    relation: relation_of(*self.graph.edge_weight(e)?),
                    directed: true,
                })
            })
            .collect();
        edges.sort_by(|a, b| {
            (&a.source, &a.target, a.relation).cmp(&(&b.source, &b.target, b.relation))
        });

        let mut import_stems: Vec<String> = self.unresolved_import_stems.iter().cloned().collect();
        import_stems.sort();

        JsonGraphDocument {
            graph: JsonGraph {
                id: rel_path(root, root.parent().unwrap_or(root)),
                graph_type: "repowise-dependency-graph".to_string(),
                label: format!("repowise dependency graph for {}", root.display()),
                directed: true,
                metadata: GraphMetadata {
                    generator: format!("repowise {}", env!("CARGO_PKG_VERSION")),
                    file_count: index.files.len(),
                    symbol_count,
                    unresolved: UnresolvedMetadata {
                        imports: self.unresolved_imports,
                        calls: self.unresolved_calls,
                        import_stems,
                        note: "Imports and calls are resolved with directory-layout heuristics, \
                               not compiler-grade name resolution. Ambiguous or external \
                               references are deliberately left unresolved rather than guessed, \
                               so they have no target node and therefore no edge in this export. \
                               The graph is partial by construction: absent edges do not imply \
                               absent dependencies."
                            .to_string(),
                    },
                },
                nodes,
                edges,
            },
        }
    }
}
