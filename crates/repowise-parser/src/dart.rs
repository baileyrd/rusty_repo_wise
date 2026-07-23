use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::{Component, Path, PathBuf};
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_dart::LANGUAGE.into())?;
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
        language: Language::Dart,
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
    /// Stack of enclosing class/mixin names, innermost last. Like
    /// Java/Kotlin (and unlike Go/C++), Dart methods are always declared
    /// directly inside their type's own body.
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
            "class_declaration" | "mixin_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let kind = if node.kind() == "mixin_declaration" {
                        SymbolKind::Mixin
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
            // A method's `signature` field is a `method_signature`
            // wrapper around the actual `function_signature` (which
            // carries the name/parameters/return-type fields);
            // `function_declaration` (only ever top-level, not nested in
            // a class body) exposes `function_signature` directly.
            "method_declaration" | "function_declaration" => {
                let func_sig = if node.kind() == "method_declaration" {
                    node.child_by_field_name("signature")
                        .and_then(|sig| sig.named_child(0))
                } else {
                    node.child_by_field_name("signature")
                };
                if let Some(func_sig) = func_sig.filter(|n| n.kind() == "function_signature") {
                    if let Some(name_node) = func_sig.child_by_field_name("name") {
                        let name = text(name_node, self.source).to_string();
                        let start_line = self.line_of(node);
                        let end_line = node.end_position().row + 1;
                        let id = Symbol::make_id(self.path, &name, start_line);
                        let parent = self.class_stack.last().cloned();
                        let body = node.child_by_field_name("body");
                        let complexity = body
                            .map(|b| {
                                metrics::cyclomatic_complexity(b, is_decision, is_nested_function)
                            })
                            .unwrap_or(0);
                        let param_count =
                            metrics::count_params(func_sig.child_by_field_name("parameters"));
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
            // Abstract/interface method signatures (no body, just a `;`)
            // wrap a `function_signature` directly, one level shallower
            // than a real `method_declaration` — still recorded as
            // symbols, 0 complexity, same treatment as Java/Kotlin/
            // Scala/PHP's bodiless methods.
            "declaration" => {
                if let Some(func_sig) = node
                    .named_child(0)
                    .filter(|n| n.kind() == "function_signature")
                {
                    if let Some(name_node) = func_sig.child_by_field_name("name") {
                        let name = text(name_node, self.source).to_string();
                        let start_line = self.line_of(node);
                        let end_line = node.end_position().row + 1;
                        let parent = self.class_stack.last().cloned();
                        let param_count =
                            metrics::count_params(func_sig.child_by_field_name("parameters"));
                        self.symbols.push(Symbol {
                            id: Symbol::make_id(self.path, &name, start_line),
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
                            complexity: 0,
                            param_count,
                            body_hash: None,
                        });
                    }
                }
            }
            "import_specification" => {
                if let Some(uri_node) = node.child_by_field_name("uri") {
                    let line = self.line_of(node);
                    let uri = text(uri_node, self.source)
                        .trim_matches(|c| c == '\'' || c == '"')
                        .to_string();
                    if !uri.is_empty() {
                        // `package:x/y.dart` (pub package) has no package
                        // registry here to resolve against — left
                        // unresolved by design, same tradeoff as TS/JS's
                        // bare npm specifiers. A plain relative path is
                        // resolved directly against the filesystem.
                        let resolved_file = (!uri.starts_with("package:"))
                            .then(|| resolve_relative_import(self.path, &uri))
                            .flatten();
                        self.imports.push(ImportRef {
                            path: uri,
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

/// For `obj.method()` (a `member_expression`) return `method`; for a
/// bare `func()`/`Widget()` (no `new` keyword required in modern Dart,
/// so construction reads as an ordinary call) return the identifier
/// itself.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "member_expression" => node
            .child_by_field_name("property")
            .map(|n| text(n, source).to_string())
            .unwrap_or_default(),
        "identifier" => text(node, source).to_string(),
        _ => String::new(),
    }
}

/// Resolve a relative import path against the importing file's own
/// directory. Dart import strings always spell out the full filename
/// (`.dart` extension included), unlike TS/JS's extension-omitting
/// convention, so only the exact path is tried.
fn resolve_relative_import(current_file: &Path, spec: &str) -> Option<PathBuf> {
    let dir = current_file.parent()?;
    let candidate = dir.join(spec);
    candidate.is_file().then(|| normalize(&candidate))
}

/// Lexically collapse `.`/`..` components (no filesystem access) so a
/// resolved import path matches the plain, already-canonical paths
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

/// Cyclomatic-complexity decision points for Dart: branches, loops,
/// exception handlers, `switch` cases (not the `default` fallback, its
/// own distinct node kind here), the ternary operator, and
/// short-circuiting boolean operators (their own distinct grammar node
/// kinds, not a shared `binary_expression`).
fn is_decision(n: Node) -> bool {
    matches!(
        n.kind(),
        "if_statement"
            | "for_statement"
            | "while_statement"
            | "do_statement"
            | "catch_clause"
            | "conditional_expression"
            | "logical_and_expression"
            | "logical_or_expression"
            | "switch_statement_case"
    )
}

/// Only nested named function/method declarations get their own symbol;
/// a closure literal passed as an argument doesn't, so its branches fold
/// into the enclosing scope's count, same tradeoff as Rust/Kotlin's
/// untracked closures.
fn is_nested_function(n: Node) -> bool {
    matches!(
        n.kind(),
        "method_declaration" | "function_declaration" | "local_function_declaration"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("test.dart"), source).unwrap()
    }

    #[test]
    fn extracts_class_mixin_and_method() {
        let rec = extract_str(
            "class Widget {\n  int area() {\n    return helper();\n  }\n}\n\nmixin Greetable {\n  void greet() {\n    print('hi');\n  }\n}\n\nabstract class Shape {\n  int area();\n}\n",
        );
        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);

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

        // `helper()` (in Widget.area) and `print('hi')` (in
        // Greetable.greet) are both real calls.
        assert_eq!(rec.calls.len(), 2);
        let helper_call = rec
            .calls
            .iter()
            .find(|c| c.callee_name == "helper")
            .unwrap();
        assert_eq!(helper_call.caller, Some(area.id.clone()));
    }

    #[test]
    fn extracts_relative_import_and_leaves_package_import_unresolved_here() {
        let rec =
            extract_str("import 'helper.dart';\nimport 'package:foo/foo.dart';\n\nclass C {}\n");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"helper.dart"));
        assert!(paths.contains(&"package:foo/foo.dart"));

        let package_import = rec
            .imports
            .iter()
            .find(|i| i.path == "package:foo/foo.dart")
            .unwrap();
        assert!(package_import.resolved_file.is_none());
    }

    #[test]
    fn records_member_and_bare_calls() {
        let rec = extract_str(
            "class C {\n  void run() {\n    this.helper();\n    obj.method();\n    var w = Widget();\n  }\n}\n",
        );
        let callees: Vec<_> = rec.calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(callees.contains(&"helper"));
        assert!(callees.contains(&"method"));
        assert!(callees.contains(&"Widget"));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_str(
            "int straightLine(int a, int b) {\n  return a + b;\n}\n\nint branchy(int x, int y, int z) {\n  if (x > 0 && y > 0) {\n    return 1;\n  } else if (z > 0) {\n    return 2;\n  }\n  for (var i = 0; i < x; i++) {\n    if (i == y) {\n      return i;\n    }\n  }\n  return 0;\n}\n",
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
            "int one(int n) {\n  var total = 0;\n  for (var i = 0; i < n; i++) {\n    total += i;\n  }\n  return total;\n}\n\nint two(int n) {\n  var total = 0;\n  for (var i = 0; i < n; i++) {\n    total += i;\n  }\n  return total;\n}\n\nint shortFn() {\n  return 1;\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "shortFn").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }
}
