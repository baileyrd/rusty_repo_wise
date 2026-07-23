use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::{Component, Path, PathBuf};
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_php::LANGUAGE_PHP.into())?;
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
        language: Language::Php,
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
    /// Stack of enclosing class/interface/trait names, innermost last.
    /// Like Java/Kotlin (and unlike Go/C++), PHP methods are always
    /// declared directly inside their type's own body.
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
            "class_declaration" | "interface_declaration" | "trait_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let kind = match node.kind() {
                        "interface_declaration" => SymbolKind::Trait,
                        "trait_declaration" => SymbolKind::Mixin,
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
            "method_declaration" | "function_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let parent = self.class_stack.last().cloned();
                    // Interface method signatures (no body, just a `;`)
                    // are still recorded, just with 0 complexity — same
                    // treatment as Java/Kotlin/Scala's bodiless methods.
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
                        param_count,
                        body_hash,
                    });
                    self.scope_stack.push(id);
                    self.visit_children(node);
                    self.scope_stack.pop();
                    return;
                }
            }
            "namespace_use_declaration" => {
                let line = self.line_of(node);
                let mut cursor = node.walk();
                let clauses: Vec<Node> = node
                    .children(&mut cursor)
                    .filter(|c| c.kind() == "namespace_use_clause")
                    .collect();
                for clause in clauses {
                    if let Some(path_text) = namespace_use_clause_path(clause, self.source) {
                        self.imports.push(ImportRef {
                            path: path_text,
                            line,
                            resolved_file: None,
                        });
                    }
                }
            }
            // `require`/`require_once`/`include`/`include_once` are four
            // distinct grammar nodes (not one node with a keyword field),
            // but all wrap a single expression the same way. Only a
            // plain (non-interpolated) string literal argument is
            // resolvable — `require __DIR__ . "/other.php"` and similar
            // concatenations are recorded with no path at all rather than
            // guessed, since evaluating `__DIR__` isn't attempted.
            "require_expression"
            | "require_once_expression"
            | "include_expression"
            | "include_once_expression" => {
                let line = self.line_of(node);
                if let Some(arg) = node.named_child(0) {
                    if let Some(path_text) = string_literal_value(arg, self.source) {
                        let resolved_file = resolve_require(self.path, &path_text);
                        self.imports.push(ImportRef {
                            path: path_text,
                            line,
                            resolved_file,
                        });
                    }
                }
            }
            "function_call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    if func.kind() == "name" {
                        self.calls.push(CallRef {
                            caller: self.current_scope(),
                            callee_name: text(func, self.source).to_string(),
                            line: self.line_of(node),
                        });
                    }
                }
            }
            "member_call_expression" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    self.calls.push(CallRef {
                        caller: self.current_scope(),
                        callee_name: text(name_node, self.source).to_string(),
                        line: self.line_of(node),
                    });
                }
            }
            // `new Type(...)`: recorded as a call to the constructed
            // class, same treatment as Java/C#/JS's object-creation
            // nodes. Dynamic instantiation (`new $className()`) has no
            // static name to record and is skipped.
            "object_creation_expression" => {
                if let Some(type_node) = node.named_child(0) {
                    if matches!(type_node.kind(), "name" | "qualified_name") {
                        self.calls.push(CallRef {
                            caller: self.current_scope(),
                            callee_name: last_type_segment(text(type_node, self.source)),
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

/// A `namespace_use_clause`'s own namespace path (`App\Models\User`),
/// distinguished from its optional `as Alias` — both can be plain
/// `name` nodes when the path itself has no namespace separator, so the
/// alias field's node id is excluded by identity, not by kind alone.
fn namespace_use_clause_path(node: Node, source: &str) -> Option<String> {
    let alias_id = node.child_by_field_name("alias").map(|n| n.id());
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|c| matches!(c.kind(), "name" | "qualified_name") && Some(c.id()) != alias_id);
    found.map(|n| text(n, source).to_string())
}

/// The literal text of a plain `string` (single-quoted) or a
/// non-interpolated `encapsed_string` (double-quoted with no `$var`/`{...}`
/// interpolation) — `None` for anything else (interpolated strings,
/// concatenation expressions, variables), since those have no static path
/// to resolve.
fn string_literal_value(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "string" => {
            let mut cursor = node.walk();
            let content = node
                .children(&mut cursor)
                .find(|c| c.kind() == "string_content");
            Some(
                content
                    .map(|c| text(c, source).to_string())
                    .unwrap_or_default(),
            )
        }
        "encapsed_string" => {
            let mut cursor = node.walk();
            let named: Vec<Node> = node.named_children(&mut cursor).collect();
            if named.len() == 1 && named[0].kind() == "string_content" {
                Some(text(named[0], source).to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Resolve a `require`/`include` path relative to the requiring file's
/// own directory — the common "included file lives next to its source"
/// convention. PHP includes typically spell out the full filename
/// (extension included), so unlike TS/JS there's no extension-guessing:
/// only the exact path is tried.
fn resolve_require(current_file: &Path, spec: &str) -> Option<PathBuf> {
    let dir = current_file.parent()?;
    let candidate = dir.join(spec);
    candidate.is_file().then(|| normalize(&candidate))
}

/// Lexically collapse `.`/`..` components (no filesystem access) so a
/// resolved require/include path matches the plain, already-canonical
/// paths `discover_files` produces.
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

/// A qualified class reference (e.g. `User` or `App\Models\User`)
/// reduced to its last segment for call-target matching.
fn last_type_segment(s: &str) -> String {
    s.rsplit('\\').next().unwrap_or(s).to_string()
}

/// Cyclomatic-complexity decision points for PHP: branches (`elseif` is
/// its own distinct `else_if_clause` node, not a nested `if_statement`),
/// loops, exception handlers, `switch` cases (not `default`, which has no
/// `value` field), the ternary operator, and short-circuiting boolean
/// operators.
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_statement"
        | "else_if_clause"
        | "for_statement"
        | "foreach_statement"
        | "while_statement"
        | "do_statement"
        | "catch_clause"
        | "conditional_expression" => true,
        "case_statement" => n.child_by_field_name("value").is_some(),
        "binary_expression" => n
            .child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Only nested method/function declarations get their own symbol; an
/// anonymous closure passed as an argument doesn't, so its branches fold
/// into the enclosing scope's count, same tradeoff as Rust/Kotlin's
/// untracked closures.
fn is_nested_function(n: Node) -> bool {
    matches!(n.kind(), "method_declaration" | "function_definition")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        let wrapped = format!("<?php\n{source}");
        extract(Path::new("Test.php"), &wrapped).unwrap()
    }

    #[test]
    fn extracts_class_interface_trait_and_method() {
        let rec = extract_str(
            "class Widget implements Shape {\n    public function area() {\n        return $this->helper();\n    }\n}\n\ninterface Shape {\n    public function area();\n}\n\ntrait Greetable {\n    public function greet() {\n        echo \"hi\";\n    }\n}\n",
        );
        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);

        let shape = rec.symbols.iter().find(|s| s.name == "Shape").unwrap();
        assert_eq!(shape.kind, SymbolKind::Trait);

        let greetable = rec.symbols.iter().find(|s| s.name == "Greetable").unwrap();
        assert_eq!(greetable.kind, SymbolKind::Mixin);

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
    fn extracts_use_statement_and_leaves_it_unresolved_here() {
        let rec = extract_str("use App\\Models\\User;\n\nclass C {}\n");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"App\\Models\\User"));
        assert!(rec.imports[0].resolved_file.is_none());
    }

    #[test]
    fn resolves_require_once_but_not_a_dir_concatenated_include() {
        let rec =
            extract_str("require_once \"helper.php\";\nrequire_once __DIR__ . \"/other.php\";\n");
        // The concatenated form has no static path and is skipped
        // entirely rather than recorded with a guessed path.
        assert_eq!(rec.imports.len(), 1);
        assert_eq!(rec.imports[0].path, "helper.php");
    }

    #[test]
    fn records_object_creation_as_a_call_to_the_class() {
        let rec = extract_str(
            "class Widget {}\n\nclass Factory {\n    public function make() {\n        return new Widget();\n    }\n}\n",
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
            "function straightLine($a, $b) {\n    return $a + $b;\n}\n\nfunction branchy($x, $y, $z) {\n    if ($x > 0 && $y > 0) {\n        return 1;\n    } elseif ($z > 0) {\n        return 2;\n    }\n    for ($i = 0; $i < $x; $i++) {\n        if ($i == $y) {\n            return $i;\n        }\n    }\n    return 0;\n}\n",
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "straightLine")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + &&(1) + elseif(1) + for(1) + if(1) = 6
        assert_eq!(branchy.complexity, 6);
        assert_eq!(branchy.param_count, 3);
    }

    #[test]
    fn hashes_duplicate_function_bodies_identically() {
        let rec = extract_str(
            "function one($n) {\n    $total = 0;\n    for ($i = 0; $i < $n; $i++) {\n        $total += $i;\n    }\n    return $total;\n}\n\nfunction two($n) {\n    $total = 0;\n    for ($i = 0; $i < $n; $i++) {\n        $total += $i;\n    }\n    return $total;\n}\n\nfunction shortFn() {\n    return 1;\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "shortFn").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }
}
