use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FieldAccessRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::{Path, PathBuf};
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse {}", path.display()))?;

    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    let mut calls = Vec::new();
    let mut field_accesses = Vec::new();

    let mut walker = Walker {
        path,
        source,
        symbols: &mut symbols,
        imports: &mut imports,
        calls: &mut calls,
        field_accesses: &mut field_accesses,
        scope_stack: Vec::new(),
        impl_type_stack: Vec::new(),
    };
    walker.visit(tree.root_node());

    Ok(FileRecord {
        path: path.to_path_buf(),
        language: Language::Rust,
        lines: source.lines().count(),
        symbols,
        imports,
        calls,
        field_accesses,
    })
}

struct Walker<'a> {
    path: &'a Path,
    source: &'a str,
    symbols: &'a mut Vec<Symbol>,
    imports: &'a mut Vec<ImportRef>,
    calls: &'a mut Vec<CallRef>,
    field_accesses: &'a mut Vec<FieldAccessRef>,
    /// Stack of enclosing symbol ids, innermost last.
    scope_stack: Vec<String>,
    /// Stack of enclosing `impl Type` names, innermost last.
    impl_type_stack: Vec<String>,
}

impl<'a> Walker<'a> {
    fn current_scope(&self) -> Option<String> {
        self.scope_stack.last().cloned()
    }

    fn line_of(&self, node: Node) -> usize {
        node.start_position().row + 1
    }

    fn visit(&mut self, node: Node) {
        match node.kind() {
            "function_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let parent = self.impl_type_stack.last().cloned();
                    let kind = if parent.is_some() {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };
                    let body = node.child_by_field_name("body");
                    let complexity = body
                        .map(|b| {
                            metrics::cyclomatic_complexity(
                                b,
                                |n| is_decision(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or(0);
                    let max_nesting_depth = body
                        .map(|b| {
                            metrics::max_nesting_depth(
                                b,
                                |n| is_decision(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or(0);
                    let param_count = metrics::count_params(node.child_by_field_name("parameters"));
                    let body_hash = body.and_then(|b| metrics::body_hash(b, self.source));
                    self.symbols.push(Symbol {
                        id: id.clone(),
                        name,
                        kind,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line,
                        parent,
                        complexity,
                        max_nesting_depth,
                        param_count,
                        body_hash,
                    });
                    self.scope_stack.push(id);
                    self.visit_children(node);
                    self.scope_stack.pop();
                    return;
                }
            }
            "struct_item" | "enum_item" | "trait_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let kind = match node.kind() {
                        "struct_item" => SymbolKind::Struct,
                        "enum_item" => SymbolKind::Enum,
                        _ => SymbolKind::Trait,
                    };
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name,
                        kind,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line,
                        parent: None,
                        complexity: 0,
                        max_nesting_depth: 0,
                        param_count: 0,
                        body_hash: None,
                    });
                }
            }
            "mod_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name: name.clone(),
                        kind: SymbolKind::Module,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line: node.end_position().row + 1,
                        parent: None,
                        complexity: 0,
                        max_nesting_depth: 0,
                        param_count: 0,
                        body_hash: None,
                    });
                    // `mod foo;` (no inline body) declares that another
                    // file defines this module. Resolve it directly via
                    // Rust's file-layout convention rather than relying on
                    // the module-index heuristic in repowise-graph.
                    if node.child_by_field_name("body").is_none() {
                        if let Some(target) = resolve_mod_file(self.path, &name) {
                            self.imports.push(ImportRef {
                                path: format!("mod {name}"),
                                line: start_line,
                                resolved_file: Some(target),
                            });
                        }
                    }
                }
            }
            "impl_item" => {
                if let Some(type_node) = node.child_by_field_name("type") {
                    let type_name = last_path_segment(text(type_node, self.source));
                    self.impl_type_stack.push(type_name);
                    self.visit_children(node);
                    self.impl_type_stack.pop();
                    return;
                }
            }
            "use_declaration" => {
                if let Some(arg) = node.child_by_field_name("argument") {
                    let mut paths = Vec::new();
                    flatten_use(arg, "", self.source, &mut paths);
                    let line = self.line_of(node);
                    for p in paths {
                        self.imports.push(ImportRef {
                            path: p,
                            line,
                            resolved_file: None,
                        });
                    }
                }
            }
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    let callee_name = call_target_name(func, self.source);
                    self.calls.push(CallRef {
                        caller: self.current_scope(),
                        callee_name,
                        line: self.line_of(node),
                    });
                }
            }
            "field_expression" => {
                if let (Some(value), Some(field)) = (
                    node.child_by_field_name("value"),
                    node.child_by_field_name("field"),
                ) {
                    if text(value, self.source) == "self" && !is_call_target(node) {
                        if let Some(method) = self.current_scope() {
                            self.field_accesses.push(FieldAccessRef {
                                method,
                                field_name: text(field, self.source).to_string(),
                                line: self.line_of(node),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        self.visit_children(node);
    }

    fn visit_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child);
        }
    }
}

/// For a call target expression, return the name that should be matched
/// against known symbol names: the identifier itself, the field name for
/// `receiver.method()`, or the last segment of a `path::to::func()`.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "identifier" => text(node, source).to_string(),
        "field_expression" => node
            .child_by_field_name("field")
            .map(|f| text(f, source).to_string())
            .unwrap_or_else(|| text(node, source).to_string()),
        "scoped_identifier" => node
            .child_by_field_name("name")
            .map(|n| text(n, source).to_string())
            .unwrap_or_else(|| last_path_segment(text(node, source))),
        _ => last_path_segment(text(node, source)),
    }
}

fn last_path_segment(s: &str) -> String {
    s.rsplit("::").next().unwrap_or(s).to_string()
}

/// True when `node` (a `field_expression`) is the `function` position of
/// its parent `call_expression` — i.e. `self.method()` rather than a
/// field read/write like `self.field`. Excluded from field-access
/// tracking so method names don't pollute the field-cohesion signal.
fn is_call_target(node: Node) -> bool {
    node.parent()
        .map(|p| {
            p.kind() == "call_expression"
                && p.child_by_field_name("function").map(|f| f.id()) == Some(node.id())
        })
        .unwrap_or(false)
}

/// Cyclomatic-complexity decision points for Rust: branches, loops, match
/// arms, and short-circuiting boolean operators (`&&` / `||`).
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_expression"
        | "if_let_expression"
        | "match_arm"
        | "while_expression"
        | "while_let_expression"
        | "loop_expression"
        | "for_expression" => true,
        "binary_expression" => n
            .child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Resolve `mod name;` to the file it declares, per Rust's file-layout
/// convention: siblings of `lib.rs`/`main.rs`/`mod.rs` live directly next
/// to it; siblings of any other `foo.rs` live under a `foo/` directory.
fn resolve_mod_file(current_file: &Path, name: &str) -> Option<PathBuf> {
    let dir = current_file.parent()?;
    let stem = current_file.file_stem()?.to_str()?;
    let base_dir = if matches!(stem, "lib" | "main" | "mod") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };

    let as_file = base_dir.join(format!("{name}.rs"));
    if as_file.is_file() {
        return Some(as_file);
    }
    let as_mod_dir = base_dir.join(name).join("mod.rs");
    if as_mod_dir.is_file() {
        return Some(as_mod_dir);
    }
    None
}

/// Recursively flatten a `use` tree node into fully dotted (`::`-joined)
/// import paths, handling grouped (`{a, b}`), aliased (`as`), and wildcard
/// (`*`) imports.
fn flatten_use(node: Node, prefix: &str, source: &str, out: &mut Vec<String>) {
    let join = |prefix: &str, seg: &str| {
        if prefix.is_empty() {
            seg.to_string()
        } else {
            format!("{prefix}::{seg}")
        }
    };
    match node.kind() {
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                flatten_use(child, prefix, source, out);
            }
        }
        "scoped_use_list" => {
            let path_part = node
                .child_by_field_name("path")
                .map(|p| path_text(p, source))
                .unwrap_or_default();
            let new_prefix = join(prefix, &path_part);
            if let Some(list) = node.child_by_field_name("list") {
                flatten_use(list, &new_prefix, source, out);
            }
        }
        "use_as_clause" => {
            if let Some(path) = node.child_by_field_name("path") {
                out.push(join(prefix, &path_text(path, source)));
            }
        }
        "use_wildcard" => {
            let path_part = node
                .child_by_field_name("path")
                .map(|p| path_text(p, source))
                .unwrap_or_default();
            out.push(format!("{}::*", join(prefix, &path_part)));
        }
        _ => {
            // identifier / scoped_identifier / self / super / crate
            out.push(join(prefix, &path_text(node, source)));
        }
    }
}

/// Convert a plain path node (identifier / scoped_identifier / self /
/// super / crate) into a single `::`-joined string.
fn path_text(node: Node, source: &str) -> String {
    match node.kind() {
        "scoped_identifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, source).to_string())
                .unwrap_or_default();
            if let Some(path) = node.child_by_field_name("path") {
                format!("{}::{}", path_text(path, source), name)
            } else {
                name
            }
        }
        _ => text(node, source).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("test.rs"), source).unwrap()
    }

    #[test]
    fn extracts_function_struct_and_method() {
        let rec = extract_str(
            r#"
            struct Foo;

            impl Foo {
                fn bar(&self) -> i32 {
                    baz()
                }
            }

            fn baz() -> i32 { 42 }
            "#,
        );
        let names: Vec<_> = rec.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"baz"));

        let bar = rec.symbols.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Method);
        assert_eq!(bar.parent.as_deref(), Some("Foo"));

        let baz = rec.symbols.iter().find(|s| s.name == "baz").unwrap();
        assert_eq!(baz.kind, SymbolKind::Function);

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "baz");
        assert_eq!(rec.calls[0].caller, Some(bar.id.clone()));
    }

    #[test]
    fn records_self_field_reads_and_writes_but_not_method_calls() {
        let rec = extract_str(
            r#"
            struct Point { x: i32, y: i32 }

            impl Point {
                fn shift(&mut self, dx: i32) -> i32 {
                    self.x += dx;
                    self.helper();
                    self.y
                }

                fn helper(&self) {}
            }
            "#,
        );
        let shift = rec.symbols.iter().find(|s| s.name == "shift").unwrap();
        let field_names: Vec<&str> = rec
            .field_accesses
            .iter()
            .filter(|f| f.method == shift.id)
            .map(|f| f.field_name.as_str())
            .collect();
        assert_eq!(field_names, vec!["x", "y"]);
        // `self.helper()` is a method call, not a field access.
        assert!(!field_names.contains(&"helper"));
    }

    #[test]
    fn flattens_grouped_use_declarations() {
        let rec = extract_str("use crate::graph::{build_graph, RepoGraph as Graph};");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"crate::graph::build_graph"));
        assert!(paths.contains(&"crate::graph::RepoGraph"));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_str(
            r#"
            fn straight_line(a: i32, b: i32) -> i32 {
                a + b
            }

            fn branchy(x: i32, y: i32, z: i32) -> i32 {
                if x > 0 && y > 0 {
                    return 1;
                } else if z > 0 {
                    return 2;
                }
                for i in 0..x {
                    if i == y {
                        return i;
                    }
                }
                0
            }
            "#,
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "straight_line")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + &&(1) + if-let-else-if(1) + for(1) + if(1) = 6
        assert_eq!(branchy.complexity, 6);
        assert_eq!(branchy.param_count, 3);
    }

    #[test]
    fn measures_nesting_depth_independently_of_cyclomatic_complexity() {
        // Same cyclomatic complexity (base + 3 ifs = 4) either way, but
        // one nests the ifs inside each other and the other keeps them
        // sequential -- nesting depth should tell them apart even though
        // complexity alone can't.
        let rec = extract_str(
            r#"
            fn sequential(x: i32) -> i32 {
                if x == 1 {
                    return 1;
                }
                if x == 2 {
                    return 2;
                }
                if x == 3 {
                    return 3;
                }
                0
            }

            fn nested(x: i32) -> i32 {
                if x > 0 {
                    if x > 10 {
                        if x > 100 {
                            return 3;
                        }
                        return 2;
                    }
                    return 1;
                }
                0
            }
            "#,
        );
        let sequential = rec.symbols.iter().find(|s| s.name == "sequential").unwrap();
        let nested = rec.symbols.iter().find(|s| s.name == "nested").unwrap();

        assert_eq!(sequential.complexity, nested.complexity);
        assert_eq!(sequential.max_nesting_depth, 1);
        assert_eq!(nested.max_nesting_depth, 3);
    }

    #[test]
    fn hashes_duplicate_function_bodies_identically() {
        let rec = extract_str(
            r#"
            fn one(n: i32) -> i32 {
                let mut total = 0;
                for i in 0..n {
                    total += i;
                }
                total
            }

            fn two(n: i32) -> i32 {
                let mut total = 0;
                for i in 0..n {
                    total += i;
                }
                total
            }

            fn short() -> i32 { 1 }
            "#,
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "short").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        // Too short to be a meaningful duplicate signal.
        assert!(short.body_hash.is_none());
    }
}
