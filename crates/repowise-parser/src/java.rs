use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::Path;
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_java::LANGUAGE.into())?;
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
        class_stack: Vec::new(),
    };
    walker.visit(tree.root_node());

    Ok(FileRecord {
        path: path.to_path_buf(),
        language: Language::Java,
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
    scope_stack: Vec<String>,
    /// Stack of enclosing class/interface/enum/record names, innermost last.
    class_stack: Vec<String>,
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
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let kind = if node.kind() == "interface_declaration" {
                        SymbolKind::Trait
                    } else {
                        SymbolKind::Class
                    };
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name: name.clone(),
                        kind,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line,
                        parent: None,
                        complexity: 0,
                        param_count: 0,
                        body_hash: None,
                    });
                    self.class_stack.push(name);
                    self.visit_children(node);
                    self.class_stack.pop();
                    return;
                }
            }
            "method_declaration" | "constructor_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let parent = self.class_stack.last().cloned();
                    // Interface method signatures / abstract methods have no
                    // body — still recorded as symbols, just with 0 complexity,
                    // same treatment as Rust trait-method signatures.
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
                    self.symbols.push(Symbol {
                        id: id.clone(),
                        name,
                        kind: SymbolKind::Method,
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
            "import_declaration" => {
                let line = self.line_of(node);
                let path_text = import_path_text(node, self.source);
                if !path_text.is_empty() {
                    self.imports.push(ImportRef {
                        path: path_text,
                        line,
                        resolved_file: None,
                    });
                }
            }
            "method_invocation" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    self.calls.push(CallRef {
                        caller: self.current_scope(),
                        callee_name: text(name_node, self.source).to_string(),
                        line: self.line_of(node),
                    });
                }
            }
            // `new Type(...)`: recorded as a call to the constructed class,
            // same treatment as JS/TS's `new_expression`.
            "object_creation_expression" => {
                if let Some(type_node) = node.child_by_field_name("type") {
                    self.calls.push(CallRef {
                        caller: self.current_scope(),
                        callee_name: last_type_segment(text(type_node, self.source)),
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

/// The dotted import path, e.g. `com.example.util.Helper` or
/// `com.example.util.Consts.MAX` for `import static ...`, `com.example.*`
/// for a wildcard import (the grammar parses the `.*` suffix as a
/// trailing `asterisk` sibling, not part of the preceding identifier, so
/// it's appended back on here). Skips the leading `import`/`static` keywords.
fn import_path_text(node: Node, source: &str) -> String {
    let mut cursor = node.walk();
    let mut path = String::new();
    let mut is_wildcard = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" | "identifier" => path = text(child, source).to_string(),
            "asterisk" => is_wildcard = true,
            _ => {}
        }
    }
    if is_wildcard {
        path.push_str(".*");
    }
    path
}

/// A generic type reference (e.g. `Foo` or `pkg.Foo<T>`) reduced to its
/// last, unparameterized segment for call-target matching.
fn last_type_segment(s: &str) -> String {
    let without_generics = s.split('<').next().unwrap_or(s);
    without_generics
        .rsplit('.')
        .next()
        .unwrap_or(without_generics)
        .to_string()
}

/// Cyclomatic-complexity decision points for Java: branches, loops
/// (including enhanced `for`), exception handlers, ternaries, `switch`
/// case labels (not `default`), and short-circuiting boolean operators.
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_statement"
        | "for_statement"
        | "enhanced_for_statement"
        | "while_statement"
        | "do_statement"
        | "catch_clause"
        | "ternary_expression" => true,
        "switch_label" => !text(n, source).trim_start().starts_with("default"),
        "binary_expression" => n
            .child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Only nested method/constructor declarations get their own symbol (Java
/// has no free-standing nested named functions, but local/anonymous
/// classes can contain their own methods) — stop recursion there so their
/// complexity isn't double-counted into the enclosing method.
fn is_nested_function(n: Node) -> bool {
    matches!(n.kind(), "method_declaration" | "constructor_declaration")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("Test.java"), source).unwrap()
    }

    #[test]
    fn extracts_class_interface_method_and_constructor() {
        let rec = extract_str(
            "public class Widget implements Shape {\n  public Widget() {}\n  public int area() {\n    return helper();\n  }\n}\n\ninterface Shape {\n  int area();\n}\n",
        );
        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);

        let shape = rec.symbols.iter().find(|s| s.name == "Shape").unwrap();
        assert_eq!(shape.kind, SymbolKind::Trait);

        let ctor = rec
            .symbols
            .iter()
            .filter(|s| s.name == "Widget" && s.kind == SymbolKind::Method)
            .count();
        assert_eq!(ctor, 1, "constructor should be recorded as a method");

        let area = rec
            .symbols
            .iter()
            .find(|s| s.name == "area" && s.parent.as_deref() == Some("Widget"))
            .unwrap();
        assert_eq!(area.kind, SymbolKind::Method);

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "helper");
        assert_eq!(rec.calls[0].caller, Some(area.id.clone()));
    }

    #[test]
    fn extracts_plain_static_and_wildcard_imports() {
        let rec = extract_str(
            "import com.example.util.Helper;\nimport static com.example.util.Consts.MAX;\nimport com.example.other.*;\n\nclass C {}\n",
        );
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"com.example.util.Helper"));
        assert!(paths.contains(&"com.example.util.Consts.MAX"));
        assert!(paths.contains(&"com.example.other.*"));
    }

    #[test]
    fn records_object_creation_as_a_call_to_the_class() {
        let rec = extract_str(
            "class Widget {}\n\nclass Factory {\n  Widget make() {\n    return new Widget();\n  }\n}\n",
        );
        let make = rec.symbols.iter().find(|s| s.name == "make").unwrap();
        let call = rec
            .calls
            .iter()
            .find(|c| c.callee_name == "Widget")
            .unwrap();
        assert_eq!(call.caller, Some(make.id.clone()));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_str(
            "class C {\n  int straightLine(int a, int b) {\n    return a + b;\n  }\n\n  int branchy(int x, int y, int z) {\n    if (x > 0 && y > 0) {\n      return 1;\n    } else if (z > 0) {\n      return 2;\n    }\n    for (int i = 0; i < x; i++) {\n      if (i == y) {\n        return i;\n      }\n    }\n    return 0;\n  }\n}\n",
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
    fn hashes_duplicate_method_bodies_identically() {
        let rec = extract_str(
            "class C {\n  int one(int n) {\n    int total = 0;\n    for (int i = 0; i < n; i++) {\n      total += i;\n    }\n    return total;\n  }\n\n  int two(int n) {\n    int total = 0;\n    for (int i = 0; i < n; i++) {\n      total += i;\n    }\n    return total;\n  }\n\n  int shortM() {\n    return 1;\n  }\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "shortM").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }

    #[test]
    fn interface_method_signature_has_no_body_but_is_still_a_symbol() {
        let rec = extract_str("interface Shape {\n  int area();\n}\n");
        let area = rec.symbols.iter().find(|s| s.name == "area").unwrap();
        assert_eq!(area.complexity, 0);
        assert_eq!(area.param_count, 0);
        assert_eq!(area.parent.as_deref(), Some("Shape"));
    }
}
