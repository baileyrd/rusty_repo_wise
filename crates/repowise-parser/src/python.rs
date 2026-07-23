use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FieldAccessRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::Path;
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into())?;
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
        class_stack: Vec::new(),
    };
    walker.visit(tree.root_node());

    Ok(FileRecord {
        path: path.to_path_buf(),
        language: Language::Python,
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
    scope_stack: Vec<String>,
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
            "function_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let parent = self.class_stack.last().cloned();
                    let kind = if parent.is_some() {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };
                    let body = node.child_by_field_name("body");
                    let complexity = body
                        .map(|b| {
                            metrics::cyclomatic_complexity(b, is_decision, |n| {
                                n.kind() == "function_definition"
                            })
                        })
                        .unwrap_or(0);
                    let max_nesting_depth = body
                        .map(|b| {
                            metrics::max_nesting_depth(b, is_decision, |n| {
                                n.kind() == "function_definition"
                            })
                        })
                        .unwrap_or(0);
                    let bumpy_road_bumps = body
                        .map(|b| {
                            metrics::bumpy_road_bumps(b, is_decision, |n| {
                                n.kind() == "function_definition"
                            })
                        })
                        .unwrap_or(0);
                    let complex_conditionals = body
                        .map(|b| {
                            metrics::complex_conditionals(
                                b,
                                condition_of,
                                is_boolean_operator,
                                |n| n.kind() == "function_definition",
                            )
                        })
                        .unwrap_or_default();
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
                        bumpy_road_bumps,
                        complex_conditionals,
                        param_count,
                        body_hash,
                    });
                    self.scope_stack.push(id);
                    self.visit_children(node);
                    self.scope_stack.pop();
                    return;
                }
            }
            "class_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name: name.clone(),
                        kind: SymbolKind::Class,
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
            "import_statement" => {
                let line = self.line_of(node);
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    match child.kind() {
                        "dotted_name" => {
                            self.imports.push(ImportRef {
                                path: text(child, self.source).to_string(),
                                line,
                                resolved_file: None,
                            });
                        }
                        "aliased_import" => {
                            if let Some(name) = child.child_by_field_name("name") {
                                self.imports.push(ImportRef {
                                    path: text(name, self.source).to_string(),
                                    line,
                                    resolved_file: None,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            "import_from_statement" => {
                let line = self.line_of(node);
                if let Some(module) = node.child_by_field_name("module_name") {
                    let module_path = text(module, self.source).to_string();
                    self.imports.push(ImportRef {
                        path: module_path.clone(),
                        line,
                        resolved_file: None,
                    });
                    let mut cursor = node.walk();
                    for child in node.named_children(&mut cursor) {
                        match child.kind() {
                            "dotted_name" if child.id() != module.id() => {
                                self.imports.push(ImportRef {
                                    path: format!("{module_path}.{}", text(child, self.source)),
                                    line,
                                    resolved_file: None,
                                });
                            }
                            "aliased_import" => {
                                if let Some(name) = child.child_by_field_name("name") {
                                    self.imports.push(ImportRef {
                                        path: format!("{module_path}.{}", text(name, self.source)),
                                        line,
                                        resolved_file: None,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "call" => {
                if let Some(func) = node.child_by_field_name("function") {
                    let callee_name = call_target_name(func, self.source);
                    self.calls.push(CallRef {
                        caller: self.current_scope(),
                        callee_name,
                        line: self.line_of(node),
                    });
                }
            }
            "attribute" => {
                if let (Some(object), Some(attribute)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("attribute"),
                ) {
                    if text(object, self.source) == "self" && !is_call_target(node) {
                        if let Some(method) = self.current_scope() {
                            self.field_accesses.push(FieldAccessRef {
                                method,
                                field_name: text(attribute, self.source).to_string(),
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

/// For `obj.method()` return `method`; for a bare `func()` return `func`.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "identifier" => text(node, source).to_string(),
        "attribute" => node
            .child_by_field_name("attribute")
            .map(|f| text(f, source).to_string())
            .unwrap_or_else(|| text(node, source).to_string()),
        _ => text(node, source)
            .rsplit('.')
            .next()
            .unwrap_or_else(|| text(node, source))
            .to_string(),
    }
}

/// True when `node` (an `attribute`) is the `function` position of its
/// parent `call` — i.e. `self.method()` rather than a field read/write
/// like `self.field`. Excluded from field-access tracking so method
/// names don't pollute the field-cohesion signal.
fn is_call_target(node: Node) -> bool {
    node.parent()
        .map(|p| {
            p.kind() == "call"
                && p.child_by_field_name("function").map(|f| f.id()) == Some(node.id())
        })
        .unwrap_or(false)
}

/// Cyclomatic-complexity decision points for Python: branches (including
/// `elif`), loops, exception handlers, ternaries, `match` cases, and
/// short-circuiting boolean operators (`and` / `or`).
fn is_decision(n: Node) -> bool {
    matches!(
        n.kind(),
        "if_statement"
            | "elif_clause"
            | "for_statement"
            | "while_statement"
            | "except_clause"
            | "conditional_expression"
            | "case_clause"
            | "boolean_operator"
    )
}

/// A short-circuiting `and`/`or` -- a separate helper from `is_decision`
/// since `complex_conditionals` counts these within one condition's own
/// subtree, not decision points across the whole function body.
fn is_boolean_operator(n: Node) -> bool {
    n.kind() == "boolean_operator"
}

/// The condition sub-expression of an `if`/`elif`/`while`.
fn condition_of(n: Node) -> Option<Node> {
    match n.kind() {
        "if_statement" | "elif_clause" | "while_statement" => n.child_by_field_name("condition"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::SymbolKind;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("test.py"), source).unwrap()
    }

    #[test]
    fn extracts_function_class_and_method() {
        let rec = extract_str(
            "def helper(x):\n    return x + 1\n\nclass Widget:\n    def render(self):\n        return helper(1)\n",
        );
        let helper = rec.symbols.iter().find(|s| s.name == "helper").unwrap();
        assert_eq!(helper.kind, SymbolKind::Function);

        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);

        let render = rec.symbols.iter().find(|s| s.name == "render").unwrap();
        assert_eq!(render.kind, SymbolKind::Method);
        assert_eq!(render.parent.as_deref(), Some("Widget"));

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "helper");
        assert_eq!(rec.calls[0].caller, Some(render.id.clone()));
    }

    #[test]
    fn records_self_field_reads_and_writes_but_not_method_calls() {
        let rec = extract_str(
            "class Point:\n    def shift(self, dx):\n        self.x += dx\n        self.helper()\n        return self.y\n\n    def helper(self):\n        pass\n",
        );
        let shift = rec.symbols.iter().find(|s| s.name == "shift").unwrap();
        let field_names: Vec<&str> = rec
            .field_accesses
            .iter()
            .filter(|f| f.method == shift.id)
            .map(|f| f.field_name.as_str())
            .collect();
        assert_eq!(field_names, vec!["x", "y"]);
        assert!(!field_names.contains(&"helper"));
    }

    #[test]
    fn extracts_import_and_from_import_paths() {
        let rec = extract_str("import os.path\nfrom pkg.utils import helper, Widget as W\n");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"os.path"));
        assert!(paths.contains(&"pkg.utils"));
        assert!(paths.contains(&"pkg.utils.helper"));
        assert!(paths.contains(&"pkg.utils.Widget"));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_str(
            "def straight_line(a, b):\n    return a + b\n\ndef branchy(x, y, z):\n    if x > 0 and y > 0:\n        return 1\n    elif z > 0:\n        return 2\n    for i in range(x):\n        if i == y:\n            return i\n    return 0\n",
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "straight_line")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + and(1) + elif(1) + for(1) + if(1) = 6
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
            "def sequential(x):\n    if x == 1:\n        return 1\n    if x == 2:\n        return 2\n    if x == 3:\n        return 3\n    return 0\n\ndef nested(x):\n    if x > 0:\n        if x > 10:\n            if x > 100:\n                return 3\n            return 2\n        return 1\n    return 0\n",
        );
        let sequential = rec.symbols.iter().find(|s| s.name == "sequential").unwrap();
        let nested = rec.symbols.iter().find(|s| s.name == "nested").unwrap();

        assert_eq!(sequential.complexity, nested.complexity);
        assert_eq!(sequential.max_nesting_depth, 1);
        assert_eq!(nested.max_nesting_depth, 3);
    }

    #[test]
    fn measures_bumpy_road_bumps_independently_of_max_nesting_depth() {
        // Both reach the same max nesting depth (2), but `scattered` has
        // three separate two-level-deep blocks while `single_deep` has
        // just one -- max_nesting_depth alone can't tell them apart, but
        // bumpy_road_bumps can.
        let rec = extract_str(
            "def scattered(x, y, z):\n    if x > 0:\n        if x > 10:\n            return 1\n    if y > 0:\n        if y > 10:\n            return 2\n    if z > 0:\n        if z > 10:\n            return 3\n    return 0\n\ndef single_deep(x):\n    if x > 0:\n        if x > 10:\n            return 1\n    return 0\n",
        );
        let scattered = rec.symbols.iter().find(|s| s.name == "scattered").unwrap();
        let single_deep = rec
            .symbols
            .iter()
            .find(|s| s.name == "single_deep")
            .unwrap();

        assert_eq!(scattered.max_nesting_depth, single_deep.max_nesting_depth);
        assert_eq!(scattered.bumpy_road_bumps, 3);
        assert_eq!(single_deep.bumpy_road_bumps, 1);
    }

    #[test]
    fn flags_conditions_chaining_three_or_more_boolean_operators() {
        let rec = extract_str(
            "def tangled(a, b, c, d):\n    if a and b and c and d:\n        return 1\n    return 0\n\ndef simple(a, b):\n    if a and b:\n        return 1\n    return 0\n",
        );
        let tangled = rec.symbols.iter().find(|s| s.name == "tangled").unwrap();
        let simple = rec.symbols.iter().find(|s| s.name == "simple").unwrap();

        assert_eq!(tangled.complex_conditionals.len(), 1);
        assert_eq!(tangled.complex_conditionals[0].operator_count, 3);
        assert!(simple.complex_conditionals.is_empty());
    }

    #[test]
    fn hashes_duplicate_function_bodies_identically() {
        let rec = extract_str(
            "def one(n):\n    total = 0\n    for i in range(n):\n        total += i\n    return total\n\ndef two(n):\n    total = 0\n    for i in range(n):\n        total += i\n    return total\n\ndef short():\n    return 1\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "short").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }
}
