use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::{Component, Path, PathBuf};
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_c::language())?;
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
        language: Language::C,
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
    // No class_stack: unlike C++, plain C has no member functions — every
    // `struct` is pure data, so structs and functions never nest.
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
            "struct_specifier" => {
                // Anonymous structs (`typedef struct { ... } Foo;`) have
                // no `name` field — skipped as a symbol, same treatment
                // as C++'s unnamed types.
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name,
                        kind: SymbolKind::Struct,
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
                }
                // Still recurse: a struct's field types can themselves be
                // nested struct/union definitions worth recording.
            }
            "function_definition" => {
                if let Some(func_declarator) = function_declarator_of(node) {
                    if let Some(declarator) = func_declarator.child_by_field_name("declarator") {
                        if matches!(declarator.kind(), "identifier") {
                            let name = text(declarator, self.source).to_string();
                            let start_line = self.line_of(node);
                            let end_line = node.end_position().row + 1;
                            let id = Symbol::make_id(self.path, &name, start_line);
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
                            let param_count = metrics::count_params(
                                func_declarator.child_by_field_name("parameters"),
                            );
                            let body_hash = body.and_then(|b| metrics::body_hash(b, self.source));
                            self.symbols.push(Symbol {
                                id: id.clone(),
                                name,
                                kind: SymbolKind::Function,
                                file: self.path.to_path_buf(),
                                start_line,
                                end_line,
                                parent: None,
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
                }
            }
            "preproc_include" => {
                if let Some(path_node) = node.child_by_field_name("path") {
                    let line = self.line_of(node);
                    let import_path = include_path_text(path_node, self.source);
                    if !import_path.is_empty() {
                        // Quote-form (`"local.h"`) is resolved directly
                        // against the filesystem, same as C++/TS-JS's
                        // relative imports; angle-form (`<stdio.h>`, kept
                        // bracketed in the stored path) has no
                        // include-path search list and is left
                        // unresolved by design.
                        let resolved_file = (!import_path.starts_with('<'))
                            .then(|| resolve_include(self.path, &import_path))
                            .flatten();
                        self.imports.push(ImportRef {
                            path: import_path,
                            line,
                            resolved_file,
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

/// A declaration's `function_declarator`, if its (possibly directly
/// present) `declarator` field is one. Doesn't unwrap pointer return-type
/// wrapping (`int *foo()`) — those declarators are skipped as a known,
/// accepted gap rather than guessed at, same tradeoff as C++.
fn function_declarator_of(node: Node) -> Option<Node> {
    let declarator = node.child_by_field_name("declarator")?;
    (declarator.kind() == "function_declarator").then_some(declarator)
}

/// For `obj.field_fn()`/`obj->field_fn()` (a `field_expression`, e.g. a
/// function pointer stored in a struct) return the field name; for a bare
/// `func()` return the identifier itself.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "field_expression" => node
            .child_by_field_name("field")
            .map(|f| text(f, source).to_string())
            .unwrap_or_default(),
        "identifier" => text(node, source).to_string(),
        _ => String::new(),
    }
}

/// `#include "local.h"` -> `local.h` (quote-form, resolvable directly
/// against the filesystem); `#include <stdio.h>` -> `<stdio.h>`
/// (angle-form, kept bracketed as a marker that it's a system/library
/// header with no include-path search to resolve against — left
/// unresolved by design).
fn include_path_text(node: Node, source: &str) -> String {
    match node.kind() {
        "string_literal" => text(node, source).trim_matches('"').to_string(),
        "system_lib_string" => text(node, source).to_string(),
        _ => String::new(),
    }
}

/// Resolve a quote-form `#include` path relative to the including file's
/// own directory — the common "local header lives next to its source"
/// convention. No project-wide include-path search list is attempted (a
/// compiler-configured `-I` list has no static file-layout equivalent), so
/// headers reached only via a search path stay unresolved.
fn resolve_include(current_file: &Path, header_name: &str) -> Option<PathBuf> {
    let dir = current_file.parent()?;
    let candidate = dir.join(header_name);
    candidate.is_file().then(|| normalize(&candidate))
}

/// Lexically collapse `.`/`..` components (no filesystem access) so a
/// resolved include path matches the plain, already-canonical paths
/// `discover_files` produces.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Cyclomatic-complexity decision points for C: branches, loops, `switch`
/// cases (not `default`), the ternary operator, and short-circuiting
/// boolean operators.
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_statement"
        | "for_statement"
        | "while_statement"
        | "do_statement"
        | "conditional_expression" => true,
        "case_statement" => !text(n, source).trim_start().starts_with("default"),
        "binary_expression" => n
            .child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Only nested named function definitions get their own symbol (C allows
/// nested functions only as a non-standard GCC extension); a function
/// pointer literal doesn't apply here at all, since C has no closures.
fn is_nested_function(n: Node) -> bool {
    n.kind() == "function_definition"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("test.c"), source).unwrap()
    }

    #[test]
    fn extracts_struct_and_function() {
        let rec = extract_str(
            "struct Widget {\n    int area;\n};\n\nint compute(struct Widget w) {\n    return helper(w.area);\n}\n",
        );
        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Struct);

        let compute = rec.symbols.iter().find(|s| s.name == "compute").unwrap();
        assert_eq!(compute.kind, SymbolKind::Function);
        assert_eq!(compute.param_count, 1);

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "helper");
        assert_eq!(rec.calls[0].caller, Some(compute.id.clone()));
    }

    #[test]
    fn extracts_quote_and_angle_includes() {
        let rec = extract_str("#include \"helper.h\"\n#include <stdio.h>\n\nvoid f(void) {}\n");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"helper.h"));
        assert!(paths.contains(&"<stdio.h>"));
    }

    #[test]
    fn records_field_and_bare_calls() {
        let rec = extract_str(
            "int topLevel(void) {\n    return helper();\n}\n\nvoid other(struct Ops *ops) {\n    ops->run();\n}\n",
        );
        let callees: Vec<_> = rec.calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(callees.contains(&"helper"));
        assert!(callees.contains(&"run"));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_str(
            "int straightLine(int a, int b) {\n    return a + b;\n}\n\nint branchy(int x, int y, int z) {\n    if (x > 0 && y > 0) {\n        return 1;\n    } else if (z > 0) {\n        return 2;\n    }\n    for (int i = 0; i < x; i++) {\n        if (i == y) {\n            return i;\n        }\n    }\n    return 0;\n}\n",
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
            "int one(int n) {\n    int total = 0;\n    for (int i = 0; i < n; i++) {\n        total += i;\n    }\n    return total;\n}\n\nint two(int n) {\n    int total = 0;\n    for (int i = 0; i < n; i++) {\n        total += i;\n    }\n    return total;\n}\n\nint shortFn(void) {\n    return 1;\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "shortFn").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }
}
