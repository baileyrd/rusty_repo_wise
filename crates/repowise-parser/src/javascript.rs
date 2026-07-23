use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FieldAccessRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::{Component, Path, PathBuf};
use tree_sitter::{Node, Parser};

/// Extensions tried, in order, when resolving a relative import/require
/// specifier that omits its extension (`./utils` -> `./utils.ts`) or a
/// directory import (`./utils` -> `./utils/index.ts`). Not a real module
/// resolver: no `package.json` "main"/"exports" handling, no `node_modules`.
const RESOLUTION_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

pub fn extract_javascript(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    extract(
        path,
        source,
        Language::JavaScript,
        tree_sitter_javascript::LANGUAGE.into(),
    )
}

pub fn extract_typescript(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let is_tsx = path.extension().and_then(|e| e.to_str()) == Some("tsx");
    let language = if is_tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    extract(path, source, Language::TypeScript, language)
}

fn extract(
    path: &Path,
    source: &str,
    language: Language,
    ts_language: tree_sitter::Language,
) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&ts_language)?;
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
        language,
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
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    self.record_function(node, name, node, None);
                    return;
                }
            }
            "method_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let parent = self.class_stack.last().cloned();
                    self.record_function(node, name, node, parent);
                    return;
                }
            }
            "class_declaration" => {
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
                        param_count: 0,
                        body_hash: None,
                    });
                    self.class_stack.push(name);
                    self.visit_children(node);
                    self.class_stack.pop();
                    return;
                }
            }
            // TypeScript-only; harmless no-op check on plain JS trees.
            "interface_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name,
                        kind: SymbolKind::Trait,
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
            // `const name = (...) => {...}` / `const name = function() {...}`:
            // a named binding to a function value is treated the same as a
            // `function name() {}` declaration. An anonymous callback passed
            // inline (not bound to a plain identifier here) gets no symbol
            // of its own; its complexity folds into the enclosing scope,
            // same as Rust's untracked closures.
            "variable_declarator" => {
                if let (Some(name_node), Some(value_node)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ) {
                    if name_node.kind() == "identifier"
                        && matches!(value_node.kind(), "arrow_function" | "function_expression")
                    {
                        let name = text(name_node, self.source).to_string();
                        let parent = self.class_stack.last().cloned();
                        self.record_function(node, name, value_node, parent);
                        return;
                    }
                }
            }
            "import_statement" => {
                if let Some(source_node) = node.child_by_field_name("source") {
                    let spec = string_value(source_node, self.source);
                    self.push_import(spec, self.line_of(node));
                }
            }
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    if func.kind() == "identifier" && text(func, self.source) == "require" {
                        if let Some(spec) = require_argument(node, self.source) {
                            self.push_import(spec, self.line_of(node));
                        }
                    } else {
                        let callee_name = call_target_name(func, self.source);
                        self.calls.push(CallRef {
                            caller: self.current_scope(),
                            callee_name,
                            line: self.line_of(node),
                        });
                    }
                }
            }
            // `new ClassName(...)`: recorded as a call to the class itself
            // so instantiated classes/constructors don't read as dead code.
            "new_expression" => {
                if let Some(ctor) = node.child_by_field_name("constructor") {
                    let callee_name = call_target_name(ctor, self.source);
                    self.calls.push(CallRef {
                        caller: self.current_scope(),
                        callee_name,
                        line: self.line_of(node),
                    });
                }
            }
            "member_expression" => {
                if let (Some(object), Some(property)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("property"),
                ) {
                    if text(object, self.source) == "this" && !is_call_target(node) {
                        if let Some(method) = self.current_scope() {
                            self.field_accesses.push(FieldAccessRef {
                                method,
                                field_name: text(property, self.source).to_string(),
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

    /// `decl_node` supplies the symbol's line span (the whole declaration,
    /// e.g. `const helper = ...;`); `func_node` supplies the parameters/body
    /// to measure and the subtree to recurse into for nested scope tracking
    /// (for a plain declaration these are the same node).
    fn record_function(
        &mut self,
        decl_node: Node,
        name: String,
        func_node: Node,
        parent: Option<String>,
    ) {
        let start_line = self.line_of(decl_node);
        let end_line = decl_node.end_position().row + 1;
        let id = Symbol::make_id(self.path, &name, start_line);
        let kind = if parent.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let body = func_node.child_by_field_name("body");
        let complexity = body
            .map(|b| {
                metrics::cyclomatic_complexity(
                    b,
                    |n| is_decision(n, self.source),
                    is_nested_function,
                )
            })
            .unwrap_or(0);
        let param_count = count_params(func_node);
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
            param_count,
            body_hash,
        });
        self.scope_stack.push(id);
        self.visit_children(func_node);
        self.scope_stack.pop();
    }

    fn push_import(&mut self, spec: String, line: usize) {
        let resolved_file = resolve_relative_import(self.path, &spec);
        self.imports.push(ImportRef {
            path: spec,
            line,
            resolved_file,
        });
    }
}

/// For `obj.method()`/`obj.prop.method()` return `method`; for a bare
/// `func()` return `func`.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "member_expression" => node
            .child_by_field_name("property")
            .map(|f| text(f, source).to_string())
            .unwrap_or_else(|| text(node, source).to_string()),
        _ => text(node, source).to_string(),
    }
}

/// True when `node` (a `member_expression`) is the target of its parent
/// `call_expression`/`new_expression` — i.e. `this.method()`/
/// `new this.Ctor()` rather than a field read/write like `this.field`.
/// Excluded from field-access tracking so method/constructor names don't
/// pollute the field-cohesion signal.
fn is_call_target(node: Node) -> bool {
    node.parent()
        .map(|p| match p.kind() {
            "call_expression" => {
                p.child_by_field_name("function").map(|f| f.id()) == Some(node.id())
            }
            "new_expression" => {
                p.child_by_field_name("constructor").map(|c| c.id()) == Some(node.id())
            }
            _ => false,
        })
        .unwrap_or(false)
}

/// A function/method's declared parameter count: the `parameters` field is
/// a list node for the normal (possibly-empty parenthesized) case, but an
/// arrow function with a single unparenthesized parameter (`x => x + 1`)
/// exposes it as a bare `parameter` field instead of a list.
fn count_params(func_node: Node) -> usize {
    if let Some(params) = func_node.child_by_field_name("parameters") {
        metrics::count_params(Some(params))
    } else if func_node.child_by_field_name("parameter").is_some() {
        1
    } else {
        0
    }
}

/// `require("./foo")`'s first argument, if it's a plain string literal.
fn require_argument(call_node: Node, source: &str) -> Option<String> {
    let args = call_node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    let first = args.named_children(&mut cursor).next()?;
    (first.kind() == "string").then(|| string_value(first, source))
}

fn string_value(node: Node, source: &str) -> String {
    text(node, source)
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

/// Cyclomatic-complexity decision points for JS/TS: branches, loops
/// (including `for...of`/`for...in`), exception handlers, ternaries,
/// switch cases (not the `default` fallback), and short-circuiting
/// boolean operators (`&&` / `||`).
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_statement" | "for_statement" | "for_in_statement" | "while_statement"
        | "do_statement" | "catch_clause" | "ternary_expression" | "switch_case" => true,
        "binary_expression" => n
            .child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Only nested *named* function declarations get their own symbol (and
/// thus their own complexity count); anonymous arrow/function-expression
/// callbacks don't, so their branches are left folded into the enclosing
/// scope's count, same tradeoff already made for Rust's untracked closures.
fn is_nested_function(n: Node) -> bool {
    n.kind() == "function_declaration"
}

/// Resolve a relative (`./...`/`../...`) import/require specifier against
/// the filesystem, trying the exact path, then each known extension, then
/// each extension as a directory `index` file. Bare specifiers (npm
/// packages) are left unresolved — no `node_modules` resolution attempted.
fn resolve_relative_import(current_file: &Path, specifier: &str) -> Option<PathBuf> {
    if !(specifier.starts_with("./") || specifier.starts_with("../")) {
        return None;
    }
    let dir = current_file.parent()?;
    let joined = dir.join(specifier);

    if joined.is_file() {
        return Some(normalize(&joined));
    }
    for ext in RESOLUTION_EXTENSIONS {
        let candidate = joined.with_extension(ext);
        if candidate.is_file() {
            return Some(normalize(&candidate));
        }
    }
    for ext in RESOLUTION_EXTENSIONS {
        let candidate = joined.join(format!("index.{ext}"));
        if candidate.is_file() {
            return Some(normalize(&candidate));
        }
    }
    None
}

/// Lexically collapse `.`/`..` components (no filesystem access) so a
/// resolved relative-import path matches the plain, already-canonical
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

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_js(source: &str) -> FileRecord {
        extract_javascript(Path::new("test.js"), source).unwrap()
    }

    fn extract_ts(source: &str) -> FileRecord {
        extract_typescript(Path::new("test.ts"), source).unwrap()
    }

    #[test]
    fn extracts_function_class_and_method() {
        let rec = extract_js(
            "function helper(x) {\n  return x + 1;\n}\n\nclass Widget {\n  render() {\n    return helper(1);\n  }\n}\n",
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
    fn records_this_field_reads_and_writes_but_not_method_calls() {
        let rec = extract_js(
            "class Point {\n  shift(dx) {\n    this.x += dx;\n    this.helper();\n    return this.y;\n  }\n\n  helper() {}\n}\n",
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
    fn records_new_expression_as_a_call_to_the_class() {
        let rec = extract_js("class Widget {}\n\nfunction make() {\n  return new Widget();\n}\n");
        let make = rec.symbols.iter().find(|s| s.name == "make").unwrap();
        let call = rec
            .calls
            .iter()
            .find(|c| c.callee_name == "Widget")
            .unwrap();
        assert_eq!(call.caller, Some(make.id.clone()));
    }

    #[test]
    fn extracts_arrow_and_function_expression_bindings_as_functions() {
        let rec = extract_js(
            "const add = (a, b) => {\n  return a + b;\n};\n\nconst named = function(x) {\n  return x;\n};\n\nconst single = x => x + 1;\n",
        );
        let add = rec.symbols.iter().find(|s| s.name == "add").unwrap();
        assert_eq!(add.kind, SymbolKind::Function);
        assert_eq!(add.param_count, 2);

        let named = rec.symbols.iter().find(|s| s.name == "named").unwrap();
        assert_eq!(named.param_count, 1);

        let single = rec.symbols.iter().find(|s| s.name == "single").unwrap();
        assert_eq!(single.param_count, 1);
    }

    #[test]
    fn extracts_esm_and_commonjs_imports() {
        let rec = extract_js(
            "import { helper, Widget as W } from \"./utils\";\nimport def from \"./default\";\nconst { x } = require(\"./other\");\n",
        );
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"./utils"));
        assert!(paths.contains(&"./default"));
        assert!(paths.contains(&"./other"));
    }

    #[test]
    fn extracts_typescript_interface_and_class() {
        let rec = extract_ts(
            "interface Shape {\n  area(): number;\n}\n\nclass Circle implements Shape {\n  area(): number {\n    return 1;\n  }\n}\n",
        );
        let shape = rec.symbols.iter().find(|s| s.name == "Shape").unwrap();
        assert_eq!(shape.kind, SymbolKind::Trait);

        let circle = rec.symbols.iter().find(|s| s.name == "Circle").unwrap();
        assert_eq!(circle.kind, SymbolKind::Class);

        let area = rec.symbols.iter().find(|s| s.name == "area").unwrap();
        assert_eq!(area.parent.as_deref(), Some("Circle"));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_js(
            "function straightLine(a, b) {\n  return a + b;\n}\n\nfunction branchy(x, y, z) {\n  if (x > 0 && y > 0) {\n    return 1;\n  } else if (z > 0) {\n    return 2;\n  }\n  for (const i of items) {\n    if (i === y) {\n      return i;\n    }\n  }\n  return 0;\n}\n",
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "straightLine")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + &&(1) + else-if(1) + for-of(1) + if(1) = 6
        assert_eq!(branchy.complexity, 6);
        assert_eq!(branchy.param_count, 3);
    }

    #[test]
    fn hashes_duplicate_function_bodies_identically() {
        let rec = extract_js(
            "function one(n) {\n  let total = 0;\n  for (let i = 0; i < n; i++) {\n    total += i;\n  }\n  return total;\n}\n\nfunction two(n) {\n  let total = 0;\n  for (let i = 0; i < n; i++) {\n    total += i;\n  }\n  return total;\n}\n\nfunction short() {\n  return 1;\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "short").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }
}
