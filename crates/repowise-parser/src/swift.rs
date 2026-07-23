use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::Path;
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_swift::LANGUAGE.into())?;
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
        language: Language::Swift,
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
    /// Stack of enclosing class/struct/enum/actor/protocol/extension
    /// names, innermost last. Like Java/Kotlin (and unlike Go/C++), Swift
    /// methods are always declared directly inside their type's own body.
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
            // `class`/`struct`/`enum`/`actor`/`extension` all share this
            // one grammar node, distinguished by the `declaration_kind`
            // field. `extension` doesn't get its own symbol (it re-opens
            // an existing type rather than declaring a new one), but its
            // name is still pushed onto `class_stack` so the extension's
            // own methods get correctly attributed to that type.
            "class_declaration" => {
                if let (Some(kind_node), Some(name_node)) = (
                    node.child_by_field_name("declaration_kind"),
                    node.child_by_field_name("name"),
                ) {
                    let name = text(name_node, self.source).to_string();
                    let is_extension = kind_node.kind() == "extension";
                    if !is_extension {
                        let start_line = self.line_of(node);
                        let end_line = node.end_position().row + 1;
                        let symbol_kind = match kind_node.kind() {
                            "struct" => SymbolKind::Struct,
                            "enum" => SymbolKind::Enum,
                            _ => SymbolKind::Class, // "class" and "actor"
                        };
                        self.symbols.push(Symbol {
                            id: Symbol::make_id(self.path, &name, start_line),
                            name: name.clone(),
                            kind: symbol_kind,
                            file: self.path.to_path_buf(),
                            start_line,
                            end_line,
                            parent: None,
                            complexity: 0,
                            max_nesting_depth: 0,
                            bumpy_road_bumps: 0,
                            complex_conditionals: Vec::new(),
                            param_count: 0,
                            primitive_param_count: 0,
                            body_hash: None,
                        });
                    }
                    self.class_stack.push(name);
                    self.visit_children(node);
                    self.class_stack.pop();
                    return;
                }
            }
            "protocol_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name: name.clone(),
                        kind: SymbolKind::Trait,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line,
                        parent: None,
                        complexity: 0,
                        max_nesting_depth: 0,
                        bumpy_road_bumps: 0,
                        complex_conditionals: Vec::new(),
                        param_count: 0,
                        primitive_param_count: 0,
                        body_hash: None,
                    });
                    self.class_stack.push(name);
                    self.visit_children(node);
                    self.class_stack.pop();
                    return;
                }
            }
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let parent = self.class_stack.last().cloned();
                    let body = node.child_by_field_name("body");
                    let complexity = body
                        .map(|b| metrics::cyclomatic_complexity(b, is_decision, is_nested_function))
                        .unwrap_or(0);
                    let max_nesting_depth = body
                        .map(|b| metrics::max_nesting_depth(b, is_decision, is_nested_function))
                        .unwrap_or(0);
                    let bumpy_road_bumps = body
                        .map(|b| metrics::bumpy_road_bumps(b, is_decision, is_nested_function))
                        .unwrap_or(0);
                    let param_count = count_params(node);
                    let body_hash = body.and_then(|b| metrics::body_hash(b, self.source));
                    self.symbols.push(Symbol {
                        id: id.clone(),
                        name,
                        kind: if parent.is_some() {
                            SymbolKind::Method
                        } else {
                            SymbolKind::Function
                        },
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line,
                        parent,
                        complexity,
                        max_nesting_depth,
                        bumpy_road_bumps,
                        complex_conditionals: Vec::new(),
                        param_count,
                        primitive_param_count: 0,
                        body_hash,
                    });
                    self.scope_stack.push(id);
                    self.visit_children(node);
                    self.scope_stack.pop();
                    return;
                }
            }
            // Protocol method *requirements* have no body at all (a
            // distinct grammar node, not `function_declaration` with an
            // absent body) — still recorded as symbols, 0 complexity,
            // same treatment as Java/Kotlin/Scala's bodiless methods.
            "protocol_function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let param_count = count_params(node);
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name,
                        kind: SymbolKind::Method,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line,
                        parent: self.class_stack.last().cloned(),
                        complexity: 0,
                        max_nesting_depth: 0,
                        bumpy_road_bumps: 0,
                        complex_conditionals: Vec::new(),
                        param_count,
                        primitive_param_count: 0,
                        body_hash: None,
                    });
                }
            }
            // Swift's `import` is module-level, not file-level — there's
            // no per-file relative-import syntax the way TS/JS/Ruby have,
            // and a module name has no direct file mapping without a full
            // build graph (package manifests, target dependencies). So
            // imports are recorded (for visibility/stats) but always left
            // unresolved by design, not a bug or an oversight.
            "import_declaration" => {
                let line = self.line_of(node);
                if let Some(module_node) = find_child(node, "identifier") {
                    let path_text = text(module_node, self.source).to_string();
                    self.imports.push(ImportRef {
                        path: path_text,
                        line,
                        resolved_file: None,
                    });
                }
            }
            "call_expression" => {
                if let Some(func) = node.child(0) {
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

/// This grammar has no wrapping `parameters` list node at all, unlike
/// every other language done so far — `parameter` nodes are direct
/// children of the `function_declaration`/`protocol_function_declaration`
/// itself, interspersed with its name/return-type/body children. Counted
/// directly rather than via the shared `metrics::count_params` helper,
/// which assumes a dedicated list node to take `named_child_count()` of.
fn count_params(node: Node) -> usize {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| c.kind() == "parameter")
        .count()
}

/// The `identifier` child of an `import_declaration` — its full text is
/// already the dotted module path (`Foundation`, `Foundation.Data`),
/// regardless of any qualifier keyword (`import struct Foundation.Data`)
/// preceding it.
fn find_child<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// For `obj.method()` (a `navigation_expression`) return `method`; for a
/// bare `func()`/`Widget()` (no `new` keyword in Swift, so construction
/// reads as an ordinary call) return the identifier itself.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "navigation_expression" => {
            let mut cursor = node.walk();
            let suffix = node
                .children(&mut cursor)
                .find(|c| c.kind() == "navigation_suffix");
            suffix
                .and_then(|s| {
                    let mut c2 = s.walk();
                    let found = s
                        .children(&mut c2)
                        .find(|c| c.kind() == "simple_identifier");
                    found.map(|n| text(n, source).to_string())
                })
                .unwrap_or_default()
        }
        "simple_identifier" => text(node, source).to_string(),
        _ => String::new(),
    }
}

/// Cyclomatic-complexity decision points for Swift: `if`/`guard`
/// conditions, loops, `switch` cases (not `default`), `catch` blocks, the
/// ternary operator, and short-circuiting boolean operators (their own
/// distinct grammar node kinds here, not a shared `binary_expression`).
fn is_decision(n: Node) -> bool {
    match n.kind() {
        "if_statement"
        | "guard_statement"
        | "for_statement"
        | "while_statement"
        | "catch_block"
        | "ternary_expression"
        | "conjunction_expression"
        | "disjunction_expression" => true,
        "switch_entry" => {
            let mut cursor = n.walk();
            let has_default = n
                .children(&mut cursor)
                .any(|c| c.kind() == "default_keyword");
            !has_default
        }
        _ => false,
    }
}

/// Only nested named function declarations get their own symbol; a
/// closure literal passed as an argument doesn't, so its branches fold
/// into the enclosing scope's count, same tradeoff as Rust/Kotlin's
/// untracked closures.
fn is_nested_function(n: Node) -> bool {
    n.kind() == "function_declaration"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("Test.swift"), source).unwrap()
    }

    #[test]
    fn extracts_class_struct_protocol_and_method() {
        let rec = extract_str(
            "class Widget {\n    func area() -> Int {\n        return helper()\n    }\n}\n\nstruct Point {\n    var x: Int\n}\n\nprotocol Shape {\n    func area() -> Int\n}\n",
        );
        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);

        let point = rec.symbols.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(point.kind, SymbolKind::Struct);

        let shape = rec.symbols.iter().find(|s| s.name == "Shape").unwrap();
        assert_eq!(shape.kind, SymbolKind::Trait);

        let area = rec
            .symbols
            .iter()
            .find(|s| s.name == "area" && s.parent.as_deref() == Some("Widget"))
            .unwrap();
        assert_eq!(area.kind, SymbolKind::Method);
        assert!(area.complexity > 0);

        let shape_area = rec
            .symbols
            .iter()
            .find(|s| s.name == "area" && s.parent.as_deref() == Some("Shape"))
            .unwrap();
        assert_eq!(shape_area.complexity, 0);

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "helper");
        assert_eq!(rec.calls[0].caller, Some(area.id.clone()));
    }

    #[test]
    fn extension_methods_are_attributed_to_the_extended_type_without_a_duplicate_symbol() {
        let rec =
            extract_str("class Widget {\n}\n\nextension Widget {\n    func extra() {\n    }\n}\n");
        let widget_count = rec.symbols.iter().filter(|s| s.name == "Widget").count();
        assert_eq!(
            widget_count, 1,
            "the extension itself shouldn't add a second Widget symbol"
        );

        let extra = rec.symbols.iter().find(|s| s.name == "extra").unwrap();
        assert_eq!(extra.parent.as_deref(), Some("Widget"));
    }

    #[test]
    fn records_module_level_imports_but_leaves_them_unresolved() {
        let rec = extract_str("import Foundation\nimport struct Foundation.Data\n\nclass C {}\n");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"Foundation"));
        assert!(paths.contains(&"Foundation.Data"));
        // Module-level imports have no per-file mapping without a full
        // build graph -- always unresolved by design, not a bug.
        assert!(rec.imports.iter().all(|i| i.resolved_file.is_none()));
    }

    #[test]
    fn records_bare_and_member_calls() {
        let rec = extract_str(
            "class Widget {\n    func make() {\n        helper()\n        obj.method()\n        let w = Widget()\n    }\n}\n",
        );
        let callees: Vec<_> = rec.calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(callees.contains(&"helper"));
        assert!(callees.contains(&"method"));
        assert!(callees.contains(&"Widget"));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_str(
            "func straightLine(a: Int, b: Int) -> Int {\n    return a + b\n}\n\nfunc branchy(x: Int, y: Int, z: Int) -> Int {\n    if x > 0 && y > 0 {\n        return 1\n    } else if z > 0 {\n        return 2\n    }\n    for i in 0..<x {\n        if i == y {\n            return i\n        }\n    }\n    return 0\n}\n",
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
            "func one(n: Int) -> Int {\n    var total = 0\n    for i in 0..<n {\n        total += i\n    }\n    return total\n}\n\nfunc two(n: Int) -> Int {\n    var total = 0\n    for i in 0..<n {\n        total += i\n    }\n    return total\n}\n\nfunc shortFn() -> Int {\n    return 1\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "shortFn").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }
}
