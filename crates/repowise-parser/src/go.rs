use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::Path;
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into())?;
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
    };
    walker.visit(tree.root_node());

    Ok(FileRecord {
        path: path.to_path_buf(),
        language: Language::Go,
        lines: source.lines().count(),
        symbols,
        imports,
        calls,
        field_accesses: Vec::new(),
    })
}

struct Walker<'a> {
    path: &'a Path,
    source: &'a str,
    symbols: &'a mut Vec<Symbol>,
    imports: &'a mut Vec<ImportRef>,
    calls: &'a mut Vec<CallRef>,
    scope_stack: Vec<String>,
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
            // `type Foo struct {...}` / `type Shape interface {...}` — Go
            // has no nested class scoping: methods are declared
            // separately (as top-level `method_declaration`s carrying a
            // receiver), never inside the type's own declaration, so
            // there's no scope_stack push/pop needed here like Java's
            // class body.
            "type_spec" => {
                if let (Some(name_node), Some(type_node)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("type"),
                ) {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let kind = match type_node.kind() {
                        "interface_type" => SymbolKind::Trait,
                        "struct_type" => SymbolKind::Struct,
                        _ => {
                            self.visit_children(node);
                            return;
                        }
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
                        param_count: 0,
                        body_hash: None,
                    });
                }
            }
            "function_declaration" | "method_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let parent = node
                        .child_by_field_name("receiver")
                        .and_then(|r| receiver_type_name(r, self.source));
                    let body = node.child_by_field_name("body");
                    let complexity = body
                        .map(|b| {
                            metrics::cyclomatic_complexity(
                                b,
                                |n| is_decision(n, self.source),
                                is_nested_function,
                            )
                        })
                        .unwrap_or(0);
                    let param_count = metrics::count_params(node.child_by_field_name("parameters"));
                    let body_hash = body.and_then(|b| metrics::body_hash(b, self.source));
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
                        complexity,
                        param_count,
                        body_hash,
                    });
                    self.scope_stack.push(id);
                    self.visit_children(node);
                    self.scope_stack.pop();
                    return;
                }
            }
            // Interface method signatures have no body — still recorded
            // as symbols (0 complexity), same treatment as every other
            // language's bodiless method signatures.
            "method_elem" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let param_count = metrics::count_params(node.child_by_field_name("parameters"));
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name,
                        kind: SymbolKind::Method,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line,
                        parent: None,
                        complexity: 0,
                        param_count,
                        body_hash: None,
                    });
                }
            }
            "import_spec" => {
                if let Some(path_node) = node.child_by_field_name("path") {
                    let line = self.line_of(node);
                    let import_path = string_literal_value(path_node, self.source);
                    if !import_path.is_empty() {
                        self.imports.push(ImportRef {
                            path: import_path,
                            line,
                            resolved_file: None,
                        });
                    }
                }
            }
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    let callee_name = call_target_name(func, self.source);
                    if !callee_name.is_empty() {
                        self.calls.push(CallRef {
                            caller: self.current_scope(),
                            callee_name,
                            line: self.line_of(node),
                        });
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

/// A method's receiver clause (`(w *Widget)` / `(w Widget)`) reduced to
/// the bare receiver type name, stripping the pointer `*` if present.
fn receiver_type_name(receiver: Node, source: &str) -> Option<String> {
    let mut cursor = receiver.walk();
    let param = receiver
        .children(&mut cursor)
        .find(|c| c.kind() == "parameter_declaration")?;
    let type_node = param.child_by_field_name("type")?;
    let type_node = if type_node.kind() == "pointer_type" {
        type_node.named_child(0)?
    } else {
        type_node
    };
    Some(text(type_node, source).to_string())
}

/// For `obj.Method()`/`pkg.Func()` (a `selector_expression`) return the
/// field/method name; for a bare `func()` return the identifier itself.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "selector_expression" => node
            .child_by_field_name("field")
            .map(|f| text(f, source).to_string())
            .unwrap_or_default(),
        "identifier" => text(node, source).to_string(),
        _ => String::new(),
    }
}

/// The unquoted contents of a Go interpreted string literal (import paths
/// are always plain double-quoted strings, never raw/backtick strings).
fn string_literal_value(node: Node, source: &str) -> String {
    text(node, source).trim_matches('"').to_string()
}

/// Cyclomatic-complexity decision points for Go: branches, `for` (Go's
/// only loop construct), `switch`/`select` case arms (not `default`),
/// and short-circuiting boolean operators.
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_statement" | "for_statement" | "expression_case" | "type_case"
        | "communication_case" => true,
        "binary_expression" => n
            .child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Only nested named function declarations get their own symbol; a
/// function literal (closure) passed as an argument doesn't, so its
/// branches fold into the enclosing scope's count, same tradeoff as
/// Rust's untracked closures.
fn is_nested_function(n: Node) -> bool {
    n.kind() == "function_declaration"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("test.go"), source).unwrap()
    }

    #[test]
    fn extracts_struct_interface_and_method() {
        let rec = extract_str(
            "package app\n\ntype Shape interface {\n\tArea() int\n}\n\ntype Widget struct {\n\tX int\n}\n\nfunc (w *Widget) Area() int {\n\treturn helper(w.X)\n}\n",
        );
        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Struct);

        let shape = rec.symbols.iter().find(|s| s.name == "Shape").unwrap();
        assert_eq!(shape.kind, SymbolKind::Trait);

        // Both the interface's method signature and the struct's actual
        // method are named "Area" — find the one with a receiver.
        let area = rec
            .symbols
            .iter()
            .find(|s| s.name == "Area" && s.parent.is_some())
            .unwrap();
        assert_eq!(area.parent.as_deref(), Some("Widget"));

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "helper");
        assert_eq!(rec.calls[0].caller, Some(area.id.clone()));
    }

    #[test]
    fn extracts_plain_and_aliased_imports() {
        let rec = extract_str(
            "package app\n\nimport (\n\t\"fmt\"\n\thelper \"example.com/mymod/util\"\n)\n",
        );
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"fmt"));
        assert!(paths.contains(&"example.com/mymod/util"));
    }

    #[test]
    fn records_selector_and_bare_calls() {
        let rec = extract_str(
            "package app\n\nfunc topLevel() int {\n\treturn helper()\n}\n\nfunc other() {\n\tfmt.Println(topLevel())\n}\n",
        );
        let callees: Vec<_> = rec.calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(callees.contains(&"helper"));
        assert!(callees.contains(&"Println"));
        assert!(callees.contains(&"topLevel"));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_str(
            "package app\n\nfunc straightLine(a int, b int) int {\n\treturn a + b\n}\n\nfunc branchy(x int, y int, z int) int {\n\tif x > 0 && y > 0 {\n\t\treturn 1\n\t} else if z > 0 {\n\t\treturn 2\n\t}\n\tfor i := 0; i < x; i++ {\n\t\tif i == y {\n\t\t\treturn i\n\t\t}\n\t}\n\treturn 0\n}\n",
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "straightLine")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + &&(1) + else-if(1) + for(1) + if(1) = 6
        assert_eq!(branchy.complexity, 6);
        assert_eq!(branchy.param_count, 3);
    }

    #[test]
    fn hashes_duplicate_function_bodies_identically() {
        let rec = extract_str(
            "package app\n\nfunc one(n int) int {\n\ttotal := 0\n\tfor i := 0; i < n; i++ {\n\t\ttotal += i\n\t}\n\treturn total\n}\n\nfunc two(n int) int {\n\ttotal := 0\n\tfor i := 0; i < n; i++ {\n\t\ttotal += i\n\t}\n\treturn total\n}\n\nfunc shortFn() int {\n\treturn 1\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "shortFn").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }

    #[test]
    fn interface_method_signature_has_no_body_but_is_still_a_symbol() {
        let rec = extract_str("package app\n\ntype Shape interface {\n\tArea() int\n}\n");
        let area = rec.symbols.iter().find(|s| s.name == "Area").unwrap();
        assert_eq!(area.complexity, 0);
    }
}
