use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::Path;
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_c_sharp::language())?;
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
        language: Language::CSharp,
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
    /// Stack of enclosing class/struct/interface names, innermost last.
    /// Unlike Go/C++, C# methods are always declared directly inside
    /// their type's body, so a simple push/pop here (same as
    /// Java/Kotlin) is enough — no need to read a receiver/qualifier.
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
            "class_declaration" | "struct_declaration" | "interface_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let kind = match node.kind() {
                        "interface_declaration" => SymbolKind::Trait,
                        "struct_declaration" => SymbolKind::Struct,
                        _ => SymbolKind::Class,
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
                        max_nesting_depth: 0,
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
                    // Interface method signatures have no body (just
                    // `;`) — still recorded as symbols (0 complexity),
                    // same treatment as every other language's bodiless
                    // method signatures.
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
                    let max_nesting_depth = body
                        .map(|b| {
                            metrics::max_nesting_depth(
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
            "using_directive" => {
                let line = self.line_of(node);
                if let Some(path_text) = using_path_text(node, self.source) {
                    self.imports.push(ImportRef {
                        path: path_text,
                        line,
                        resolved_file: None,
                    });
                }
            }
            "invocation_expression" => {
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
            // `new Type(...)`: recorded as a call to the constructed
            // class, same treatment as JS/TS/Java's `new` handling.
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

/// The target namespace/type path of a `using` directive: the plain
/// path for `using System;`/`using App.Util;`, or the aliased target
/// (the right-hand side) for `using Helper = App.Util.Helper;` —
/// `using static ...` is skipped (its target is a type's static members,
/// not a namespace/file this port's resolution model can point at).
fn using_path_text(node: Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    if node.children(&mut cursor).any(|c| c.kind() == "static") {
        return None;
    }
    if let Some(name_node) = node.child_by_field_name("name") {
        // Aliased form: the alias is field "name"; the target follows
        // the `=` as the last named child.
        let mut cursor = node.walk();
        let target = node
            .children(&mut cursor)
            .filter(|c| matches!(c.kind(), "identifier" | "qualified_name"))
            .find(|c| c.id() != name_node.id());
        return target.map(|t| text(t, source).to_string());
    }
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|c| matches!(c.kind(), "identifier" | "qualified_name"));
    found.map(|c| text(c, source).to_string())
}

/// For `obj.Method()` (a `member_access_expression`) return the member
/// name; for a bare `Func()` return the identifier itself.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "member_access_expression" => node
            .child_by_field_name("name")
            .map(|n| text(n, source).to_string())
            .unwrap_or_default(),
        "identifier" => text(node, source).to_string(),
        _ => String::new(),
    }
}

fn last_type_segment(s: &str) -> String {
    let without_generics = s.split('<').next().unwrap_or(s);
    without_generics
        .rsplit('.')
        .next()
        .unwrap_or(without_generics)
        .to_string()
}

/// Cyclomatic-complexity decision points for C#: branches, loops
/// (including `foreach`), exception handlers, `switch` sections (not
/// `default`), the ternary operator, and short-circuiting boolean
/// operators.
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_statement"
        | "for_statement"
        | "foreach_statement"
        | "while_statement"
        | "do_statement"
        | "catch_clause"
        | "conditional_expression" => true,
        "switch_section" => !text(n, source).trim_start().starts_with("default"),
        "binary_expression" => n
            .child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Only nested named method/constructor declarations get their own
/// symbol; a lambda passed as an argument doesn't, so its branches fold
/// into the enclosing scope's count, same tradeoff as Rust's untracked
/// closures.
fn is_nested_function(n: Node) -> bool {
    matches!(n.kind(), "method_declaration" | "constructor_declaration")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("Test.cs"), source).unwrap()
    }

    #[test]
    fn extracts_class_interface_and_method() {
        let rec = extract_str(
            "class Widget : IShape {\n  public Widget() {}\n  public int Area() {\n    return Helper();\n  }\n}\n\ninterface IShape {\n  int Area();\n}\n",
        );
        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);

        let shape = rec.symbols.iter().find(|s| s.name == "IShape").unwrap();
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
            .find(|s| s.name == "Area" && s.parent.as_deref() == Some("Widget"))
            .unwrap();
        assert_eq!(area.kind, SymbolKind::Method);

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "Helper");
        assert_eq!(rec.calls[0].caller, Some(area.id.clone()));
    }

    #[test]
    fn extracts_plain_dotted_and_aliased_usings() {
        let rec = extract_str(
            "using System;\nusing App.Util;\nusing Helper = App.Util.Helper;\nusing static System.Math;\n\nclass C {}\n",
        );
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"System"));
        assert!(paths.contains(&"App.Util"));
        assert!(paths.contains(&"App.Util.Helper"));
        // `using static ...` is skipped — not a namespace/file target.
        assert!(!paths.iter().any(|p| p.contains("Math")));
    }

    #[test]
    fn records_object_creation_as_a_call_to_the_class() {
        let rec = extract_str(
            "class Widget {}\n\nclass Factory {\n  Widget Make() {\n    return new Widget();\n  }\n}\n",
        );
        let make = rec.symbols.iter().find(|s| s.name == "Make").unwrap();
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
            "class C {\n  int StraightLine(int a, int b) {\n    return a + b;\n  }\n\n  int Branchy(int x, int y, int z) {\n    if (x > 0 && y > 0) {\n      return 1;\n    } else if (z > 0) {\n      return 2;\n    }\n    for (int i = 0; i < x; i++) {\n      if (i == y) {\n        return i;\n      }\n    }\n    return 0;\n  }\n}\n",
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "StraightLine")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "Branchy").unwrap();
        // base(1) + if(1) + &&(1) + else-if(1) + for(1) + if(1) = 6
        assert_eq!(branchy.complexity, 6);
        assert_eq!(branchy.param_count, 3);
    }

    #[test]
    fn hashes_duplicate_method_bodies_identically() {
        let rec = extract_str(
            "class C {\n  int One(int n) {\n    int total = 0;\n    for (int i = 0; i < n; i++) {\n      total += i;\n    }\n    return total;\n  }\n\n  int Two(int n) {\n    int total = 0;\n    for (int i = 0; i < n; i++) {\n      total += i;\n    }\n    return total;\n  }\n\n  int Short() {\n    return 1;\n  }\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "One").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "Two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "Short").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }

    #[test]
    fn interface_method_signature_has_no_body_but_is_still_a_symbol() {
        let rec = extract_str("interface IShape {\n  int Area();\n}\n");
        let area = rec.symbols.iter().find(|s| s.name == "Area").unwrap();
        assert_eq!(area.complexity, 0);
        assert_eq!(area.parent.as_deref(), Some("IShape"));
    }
}
