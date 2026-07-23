use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::{Component, Path, PathBuf};
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_ruby::LANGUAGE.into())?;
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
        language: Language::Ruby,
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
    /// Stack of enclosing class/module names, innermost last. Like
    /// Java/Kotlin (and unlike Go/C++), Ruby methods are always declared
    /// directly inside their `class`/`module`'s own body, so a simple
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
            "class" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    self.visit_type_def(node, name_node, SymbolKind::Class);
                    return;
                }
            }
            "module" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    self.visit_type_def(node, name_node, SymbolKind::Module);
                    return;
                }
            }
            // `def self.foo` (a class-level method): same nesting and
            // recording treatment as an ordinary instance `method`, just a
            // distinct grammar node since it carries an explicit receiver
            // (usually `self`).
            "method" | "singleton_method" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let parent = self.class_stack.last().cloned();
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
                        primitive_param_count: 0,
                        body_hash,
                    });
                    self.scope_stack.push(id);
                    self.visit_children(node);
                    self.scope_stack.pop();
                    return;
                }
            }
            "call" => {
                let method_name = node
                    .child_by_field_name("method")
                    .map(|n| text(n, self.source))
                    .unwrap_or("");
                if matches!(method_name, "require_relative" | "require") {
                    if let Some(spec) = string_argument(node, self.source) {
                        let line = self.line_of(node);
                        // `require_relative` is always file-relative and
                        // resolvable directly against the filesystem;
                        // plain `require` is gem-based ($LOAD_PATH), which
                        // has no static equivalent to resolve against, so
                        // it's left unresolved by design, same tradeoff as
                        // C++'s angle-form `#include`/TS-JS's bare
                        // specifiers.
                        let resolved_file = (method_name == "require_relative")
                            .then(|| resolve_require_relative(self.path, &spec))
                            .flatten();
                        self.imports.push(ImportRef {
                            path: spec,
                            line,
                            resolved_file,
                        });
                    }
                } else {
                    let callee_name = call_target_name(node, self.source);
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

    fn visit_type_def(&mut self, node: Node, name_node: Node, kind: SymbolKind) {
        let name = simple_name(name_node, self.source);
        let start_line = self.line_of(node);
        let end_line = node.end_position().row + 1;
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
            primitive_param_count: 0,
            body_hash: None,
        });
        self.class_stack.push(name);
        self.visit_children(node);
        self.class_stack.pop();
    }

    fn visit_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child);
        }
    }
}

/// A `class`/`module`'s own name: usually a plain `constant` (`Widget`),
/// but a reopened nested definition (`class Foo::Bar`) parses as a
/// `scope_resolution` — reduced to its last segment, same treatment as
/// C++'s qualified out-of-class method names.
fn simple_name(name_node: Node, source: &str) -> String {
    let full = text(name_node, source);
    full.rsplit("::").next().unwrap_or(full).to_string()
}

/// For `receiver.method(...)`/`method(...)` return the method name — except
/// `receiver.new` (a constructor call, Ruby's equivalent of `new Type()`),
/// which is recorded as a call to the receiver class itself so instantiated
/// classes don't read as dead code.
fn call_target_name(node: Node, source: &str) -> String {
    let method_name = node
        .child_by_field_name("method")
        .map(|n| text(n, source))
        .unwrap_or("");
    if method_name == "new" {
        if let Some(receiver) = node.child_by_field_name("receiver") {
            if receiver.kind() == "constant" {
                return text(receiver, source).to_string();
            }
        }
    }
    method_name.to_string()
}

/// A call's first argument, if it's a plain (non-interpolated) string
/// literal — good enough for `require`/`require_relative`'s always-literal
/// argument.
fn string_argument(call_node: Node, source: &str) -> Option<String> {
    let args = call_node.child_by_field_name("arguments")?;
    let first = args.named_child(0)?;
    if first.kind() != "string" {
        return None;
    }
    let mut cursor = first.walk();
    let content = first
        .children(&mut cursor)
        .find(|c| c.kind() == "string_content")?;
    Some(text(content, source).to_string())
}

/// Resolve a `require_relative` specifier against the filesystem, relative
/// to the requiring file's own directory — trying the exact path, then
/// with a `.rb` extension appended (the common omitted-extension form).
/// No `$LOAD_PATH` search is attempted (that's `require`'s job, and has no
/// static file-layout equivalent).
fn resolve_require_relative(current_file: &Path, spec: &str) -> Option<PathBuf> {
    let dir = current_file.parent()?;
    let joined = dir.join(spec);
    if joined.is_file() {
        return Some(normalize(&joined));
    }
    let with_ext = joined.with_extension("rb");
    if with_ext.is_file() {
        return Some(normalize(&with_ext));
    }
    None
}

/// Lexically collapse `.`/`..` components (no filesystem access) so a
/// resolved `require_relative` path matches the plain, already-canonical
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

/// Cyclomatic-complexity decision points for Ruby: `if`/`unless`/`elsif`
/// branches (including their statement-modifier forms), loops, `rescue`
/// handlers, `case`/`when` clauses, the ternary operator, and
/// short-circuiting boolean operators.
///
/// The grammar names several rules after their own bare keyword (`if`,
/// `elsif`, `while`, `until`, `for`, `rescue`, `when`), and *also* keeps
/// that keyword as an anonymous child token of the identical kind string
/// — so without the `is_named()` guard, each of those would be counted
/// twice (once for the rule, once for its own keyword token).
fn is_decision(n: Node, source: &str) -> bool {
    if !n.is_named() {
        return false;
    }
    match n.kind() {
        "if" | "unless" | "elsif" | "while" | "until" | "for" | "rescue" | "when"
        | "conditional" | "if_modifier" | "unless_modifier" | "while_modifier"
        | "until_modifier" => true,
        "binary" => n
            .child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Only nested `def`/`def self.` declarations get their own symbol; a
/// block passed to another method (`each { ... }`) doesn't, so its
/// branches fold into the enclosing scope's count, same tradeoff as
/// Rust/Kotlin's untracked closures.
fn is_nested_function(n: Node) -> bool {
    matches!(n.kind(), "method" | "singleton_method")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("test.rb"), source).unwrap()
    }

    #[test]
    fn extracts_class_module_and_method() {
        let rec = extract_str(
            "class Widget\n  def area\n    helper()\n  end\nend\n\nmodule Utils\n  def self.helper\n  end\nend\n",
        );
        let widget = rec.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);

        let utils = rec.symbols.iter().find(|s| s.name == "Utils").unwrap();
        assert_eq!(utils.kind, SymbolKind::Module);

        let area = rec
            .symbols
            .iter()
            .find(|s| s.name == "area" && s.parent.as_deref() == Some("Widget"))
            .unwrap();
        assert_eq!(area.kind, SymbolKind::Method);

        let helper = rec
            .symbols
            .iter()
            .find(|s| s.name == "helper" && s.parent.as_deref() == Some("Utils"))
            .unwrap();
        assert_eq!(helper.kind, SymbolKind::Method);

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "helper");
        assert_eq!(rec.calls[0].caller, Some(area.id.clone()));
    }

    #[test]
    fn extracts_require_relative_and_require() {
        let rec = extract_str("require_relative \"helper\"\nrequire \"json\"\n\nclass C\nend\n");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"helper"));
        assert!(paths.contains(&"json"));

        // `require_relative` is resolved against the filesystem at parse
        // time in the graph layer, not here; but plain `require` never
        // gets a `resolved_file` at all, gem resolution being out of scope.
        let require_json = rec.imports.iter().find(|i| i.path == "json").unwrap();
        assert!(require_json.resolved_file.is_none());
    }

    #[test]
    fn records_constructor_call_as_a_call_to_the_class() {
        let rec = extract_str(
            "class Widget\nend\n\nclass Factory\n  def make\n    Widget.new\n  end\nend\n",
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
            "def straight_line(a, b)\n  a + b\nend\n\ndef branchy(x, y, z)\n  if x > 0 && y > 0\n    1\n  elsif z > 0\n    2\n  else\n    total = 0\n    i = 0\n    while i < x\n      total += i\n      i += 1\n    end\n    total\n  end\nend\n",
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "straight_line")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + &&(1) + elsif(1) + while(1) = 5
        assert_eq!(branchy.complexity, 5);
        assert_eq!(branchy.param_count, 3);
    }

    #[test]
    fn hashes_duplicate_method_bodies_identically() {
        let rec = extract_str(
            "def one(n)\n  total = 0\n  i = 0\n  while i < n\n    total += i\n    i += 1\n  end\n  total\nend\n\ndef two(n)\n  total = 0\n  i = 0\n  while i < n\n    total += i\n    i += 1\n  end\n  total\nend\n\ndef short_m\n  1\nend\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "short_m").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }
}
