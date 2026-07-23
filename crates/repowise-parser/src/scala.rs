use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::Path;
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_scala::LANGUAGE.into())?;
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
        language: Language::Scala,
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
    /// Stack of enclosing class/object/trait names, innermost last. Like
    /// Java/Kotlin (and unlike Go/C++), Scala methods are always declared
    /// directly inside their type's `template_body`, so a simple
    /// push/pop here is enough.
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
            "class_definition" | "object_definition" | "trait_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let kind = if node.kind() == "trait_definition" {
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
                        max_nesting_depth: 0,
                        bumpy_road_bumps: 0,
                        complex_conditionals: Vec::new(),
                        param_count: 0,
                        body_hash: None,
                    });
                    self.class_stack.push(name);
                    self.visit_children(node);
                    self.class_stack.pop();
                    return;
                }
            }
            // Bodiless `def` signatures (abstract methods in traits) parse
            // as a distinct `function_declaration` node with no `body`
            // field at all, rather than `function_definition` with an
            // absent body — still recorded as symbols, 0 complexity, same
            // treatment as Java/Kotlin's bodiless methods.
            "function_definition" | "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let parent = self.class_stack.last().cloned();
                    // Abstract method signatures in traits (no `= ...`
                    // body) are still recorded, just with 0 complexity —
                    // same treatment as Java/Kotlin's bodiless methods.
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
                    let bumpy_road_bumps = body
                        .map(|b| {
                            metrics::bumpy_road_bumps(
                                b,
                                |n| is_decision(n, self.source),
                                is_nested_function,
                            )
                        })
                        .unwrap_or(0);
                    // Scala allows curried multi-parameter-list defs
                    // (`def f(a: Int)(b: Int)`); only the first list is
                    // counted, an accepted simplification.
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
                        bumpy_road_bumps,
                        complex_conditionals: Vec::new(),
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
            // `new Type(...)`: recorded as a call to the constructed
            // class, same treatment as Java/C#'s object-creation nodes.
            "instance_expression" => {
                if let Some(type_node) = node.named_child(0) {
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

/// The dotted import path, e.g. `scala.collection.mutable.Map`, or
/// `scala.collection.mutable._` for a wildcard import (the grammar
/// parses the wildcard as a separate `namespace_wildcard` sibling, not
/// part of the `path` field, so it's appended back on here). Grouped
/// selector imports (`import foo.{Bar, Baz}`) are recorded as a single
/// path to the enclosing package (`foo`), not expanded per-selector — an
/// accepted simplification, same tradeoff as other languages' wildcard
/// handling.
fn import_path_text(node: Node, source: &str) -> String {
    let mut cursor = node.walk();
    let mut segments = Vec::new();
    for child in node.children_by_field_name("path", &mut cursor) {
        if matches!(child.kind(), "identifier" | "operator_identifier") {
            segments.push(text(child, source).to_string());
        }
    }
    let mut path = segments.join(".");
    let mut cursor = node.walk();
    if node
        .children(&mut cursor)
        .any(|c| c.kind() == "namespace_wildcard")
    {
        path.push_str("._");
    }
    path
}

/// For `obj.method()` (a `field_expression`) return `method`; for a bare
/// `func()`/`Widget()` return the identifier itself; for a
/// type-parameterized call (`func[T]()`, a `generic_function`) unwrap to
/// the underlying function.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "field_expression" => node
            .child_by_field_name("field")
            .map(|n| text(n, source).to_string())
            .unwrap_or_default(),
        "generic_function" => node
            .child_by_field_name("function")
            .map(|f| call_target_name(f, source))
            .unwrap_or_default(),
        "identifier" => text(node, source).to_string(),
        _ => String::new(),
    }
}

/// A generic type reference (e.g. `Widget` or `pkg.Widget[T]`) reduced to
/// its last, unparameterized segment for call-target matching.
fn last_type_segment(s: &str) -> String {
    let without_generics = s.split('[').next().unwrap_or(s);
    without_generics
        .rsplit('.')
        .next()
        .unwrap_or(without_generics)
        .to_string()
}

/// Cyclomatic-complexity decision points for Scala: `if`/`while`/`do-while`/
/// `for` expressions, exception handlers, `match` case clauses (not the
/// bare `_` wildcard fallthrough), and short-circuiting boolean operators.
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_expression"
        | "while_expression"
        | "do_while_expression"
        | "for_expression"
        | "catch_clause" => true,
        "case_clause" => n
            .child_by_field_name("pattern")
            .map(|p| text(p, source).trim() != "_")
            .unwrap_or(true),
        "infix_expression" => n
            .child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Only nested `def` declarations get their own symbol; a lambda passed
/// as an argument doesn't, so its branches fold into the enclosing
/// scope's count, same tradeoff as Rust/Kotlin's untracked closures.
fn is_nested_function(n: Node) -> bool {
    matches!(n.kind(), "function_definition" | "function_declaration")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("Test.scala"), source).unwrap()
    }

    #[test]
    fn extracts_class_trait_object_and_method() {
        let rec = extract_str(
            "class Widget extends Shape {\n  def area(): Int = {\n    helper()\n  }\n}\n\ntrait Shape {\n  def area(): Int\n}\n\nobject Registry {\n  def get(): Unit = {}\n}\n",
        );
        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);

        let shape = rec.symbols.iter().find(|s| s.name == "Shape").unwrap();
        assert_eq!(shape.kind, SymbolKind::Trait);

        let registry = rec.symbols.iter().find(|s| s.name == "Registry").unwrap();
        assert_eq!(registry.kind, SymbolKind::Class);

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
    fn extracts_plain_and_wildcard_imports() {
        let rec = extract_str(
            "import scala.collection.mutable.Map\nimport scala.collection.mutable._\n\nclass C\n",
        );
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"scala.collection.mutable.Map"));
        assert!(paths.contains(&"scala.collection.mutable._"));
    }

    #[test]
    fn records_object_creation_as_a_call_to_the_class() {
        let rec = extract_str(
            "class Widget\n\nclass Factory {\n  def make(): Widget = {\n    new Widget()\n  }\n}\n",
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
            "class C {\n  def straightLine(a: Int, b: Int): Int = {\n    a + b\n  }\n\n  def branchy(x: Int, y: Int, z: Int): Int = {\n    if (x > 0 && y > 0) {\n      1\n    } else if (z > 0) {\n      2\n    } else {\n      var total = 0\n      var i = 0\n      while (i < x) {\n        total += i\n        i += 1\n      }\n      total\n    }\n  }\n}\n",
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "straightLine")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + &&(1) + else-if(1) + while(1) = 5
        assert_eq!(branchy.complexity, 5);
        assert_eq!(branchy.param_count, 3);
    }

    #[test]
    fn hashes_duplicate_method_bodies_identically() {
        let rec = extract_str(
            "class C {\n  def one(n: Int): Int = {\n    var total = 0\n    var i = 0\n    while (i < n) {\n      total += i\n      i += 1\n    }\n    total\n  }\n\n  def two(n: Int): Int = {\n    var total = 0\n    var i = 0\n    while (i < n) {\n      total += i\n      i += 1\n    }\n    total\n  }\n\n  def shortM(): Int = {\n    1\n  }\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "shortM").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }

    #[test]
    fn trait_method_signature_has_no_body_but_is_still_a_symbol() {
        let rec = extract_str("trait Shape {\n  def area(): Int\n}\n");
        let area = rec.symbols.iter().find(|s| s.name == "area").unwrap();
        assert_eq!(area.complexity, 0);
        assert_eq!(area.param_count, 0);
        assert_eq!(area.parent.as_deref(), Some("Shape"));
    }
}
