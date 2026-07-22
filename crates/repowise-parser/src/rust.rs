use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::{Path, PathBuf};
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse {}", path.display()))?;

    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    let mut calls = Vec::new();

    let mut walker = Walker {
        path,
        source,
        symbols: &mut symbols,
        imports: &mut imports,
        calls: &mut calls,
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
    })
}

struct Walker<'a> {
    path: &'a Path,
    source: &'a str,
    symbols: &'a mut Vec<Symbol>,
    imports: &'a mut Vec<ImportRef>,
    calls: &'a mut Vec<CallRef>,
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
                    self.symbols.push(Symbol {
                        id: id.clone(),
                        name,
                        kind,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line,
                        parent,
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
    fn flattens_grouped_use_declarations() {
        let rec = extract_str("use crate::graph::{build_graph, RepoGraph as Graph};");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"crate::graph::build_graph"));
        assert!(paths.contains(&"crate::graph::RepoGraph"));
    }
}
