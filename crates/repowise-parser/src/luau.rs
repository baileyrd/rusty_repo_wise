//! Luau/Roblox support (issue #341), the port's first grammar sourced from
//! the `tree-sitter-grammars` community org rather than a language's own
//! canonical maintainers -- same "Full tier" extraction depth as every
//! other tree-sitter-backed language here, not the shallower "Partial"
//! tier upstream repowise defines for it: Luau's real, actively-maintained
//! `tree-sitter-luau` grammar makes the deeper extraction viable, so this
//! port doesn't need a separate tier concept just for this one language.
//!
//! Luau is Roblox's Lua 5.1 dialect plus optional type annotations, string
//! interpolation, and a few other extensions. Node-kind names below were
//! grounded empirically against `tree-sitter-luau` 1.2.0's own
//! `node-types.json`/`grammar.json` and a throwaway example parsing real
//! snippets (`tree.root_node().to_sexp()`), not guessed from memory or
//! from other Lua grammars -- several shapes are easy to get wrong
//! otherwise, e.g.:
//! - `function foo() end` and `local function foo() end` both parse to the
//!   *same* `function_declaration` node kind (the `local` keyword is just
//!   an extra leading anonymous token), so there's no need to special-case
//!   "local" at all for symbol extraction -- only the `name` field's node
//!   kind (`identifier` vs `dot_index_expression` vs
//!   `method_index_expression`) matters, for telling a plain function from
//!   a dot/colon-defined method.
//! - `binary_expression` has no `operator` field (unlike Go/Kotlin/JS) --
//!   the operator is a positional anonymous token between the `left` and
//!   `right` fields, reached via `node.child(1)`.
//!
//! Imports: Luau has no `import`/`use` keyword. The idiomatic mechanism is
//! `local Foo = require(path)`, where `path` is either a Roblox
//! instance-tree dot-path (`require(script.Parent.Foo)`, a bare
//! `dot_index_expression`/`identifier`, not a string) or a plain string
//! path (`require("Bar")`) in non-Roblox Luau. Handled the same shape as
//! JavaScript's CommonJS `require()` in `javascript.rs`: any
//! `function_call` whose callee is the bare identifier `require` is
//! recorded as an `ImportRef` instead of a `CallRef`, regardless of where
//! in an expression it appears (so it's caught whether or not it's the
//! right-hand side of a `local` binding). Unlike JS's relative-path
//! resolution, `resolved_file` is always left `None` here: a Roblox
//! instance-tree path like `script.Parent.Foo` doesn't correspond to a
//! filesystem path at all (Roblox's `script` is a runtime instance
//! reference, not a source-tree location this port can see), and a plain
//! string path has no fixed extension/directory convention to resolve
//! against the way JS's `./` imports do. This is a real, documented
//! resolution gap, not an oversight -- `repowise-graph`'s module-map
//! heuristic simply won't connect Luau `require` edges to their target
//! files.
//!
//! Symbols: `function name(...)`/`local function name(...)` at top level
//! become `Function`s; `function obj.method(...)`/`function
//! obj:method(...)` become `Method`s, attributed to `obj` (the `table`
//! field's source text -- for a dotted receiver like `A.B.method`, that's
//! `A.B` verbatim, not further decomposed) the same way Go attributes a
//! method to its receiver type. Luau has no class/struct/interface/trait
//! *syntax* -- "classes" are a table + metatable idiom, not a construct
//! tree-sitter's grammar recognizes structurally -- so unlike Kotlin/Go
//! this module never emits `SymbolKind::Class`/`Trait`/`Struct`; a
//! `local Widget = {}` table used as a class is invisible to symbol
//! extraction, only its `function Widget:method()` methods are.
//!
//! Field accesses: left empty (`field_accesses: Vec::new()`), the same
//! depth Kotlin's and Go's own extractors stop at -- not implemented
//! deeper just because JS/TS happens to track `this.field` reads.
//!
//! Calls: ordinary call-expression extraction, same shape as every other
//! language -- `function_call`'s `name` field is either a bare
//! `identifier`, a `dot_index_expression` (qualified call, last segment
//! used as the callee name), or a `method_index_expression` (colon call,
//! its `method` field used as the callee name).

use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::Path;
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_luau::LANGUAGE.into())?;
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
        language: Language::Luau,
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
                    let (name, parent) = match name_node.kind() {
                        "identifier" => (text(name_node, self.source).to_string(), None),
                        "dot_index_expression" => {
                            let field = name_node.child_by_field_name("field");
                            let table = name_node.child_by_field_name("table");
                            match (field, table) {
                                (Some(f), Some(t)) => (
                                    text(f, self.source).to_string(),
                                    Some(text(t, self.source).to_string()),
                                ),
                                _ => {
                                    self.visit_children(node);
                                    return;
                                }
                            }
                        }
                        "method_index_expression" => {
                            let method = name_node.child_by_field_name("method");
                            let table = name_node.child_by_field_name("table");
                            match (method, table) {
                                (Some(m), Some(t)) => (
                                    text(m, self.source).to_string(),
                                    Some(text(t, self.source).to_string()),
                                ),
                                _ => {
                                    self.visit_children(node);
                                    return;
                                }
                            }
                        }
                        _ => {
                            self.visit_children(node);
                            return;
                        }
                    };
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let kind = if parent.is_some() {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };
                    let body = node.child_by_field_name("body");
                    let params = node.child_by_field_name("parameters");
                    let complexity = body
                        .map(|b| metrics::cyclomatic_complexity(b, is_decision, is_nested_function))
                        .unwrap_or(0);
                    let max_nesting_depth = body
                        .map(|b| metrics::max_nesting_depth(b, is_decision, is_nested_function))
                        .unwrap_or(0);
                    let bumpy_road_bumps = body
                        .map(|b| metrics::bumpy_road_bumps(b, is_decision, is_nested_function))
                        .unwrap_or(0);
                    let complex_conditionals = body
                        .map(|b| {
                            metrics::complex_conditionals(
                                b,
                                condition_of,
                                is_boolean_operator,
                                is_nested_function,
                            )
                        })
                        .unwrap_or_default();
                    let param_count = metrics::count_params(params);
                    let primitive_param_count = metrics::primitive_param_count(
                        params,
                        |n| param_type(n, self.source),
                        is_primitive_type,
                    );
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
                        io_in_loop: Vec::new(),
                        string_concat_in_loop: Vec::new(),
                        resource_construction_in_loop: Vec::new(),
                        lock_in_loop: Vec::new(),
                        list_insert_zero_in_loop: Vec::new(),
                        json_parse_in_loop: Vec::new(),
                        regex_compile_in_loop: Vec::new(),
                        nested_loop_with_io: Vec::new(),
                        nested_loop_quadratic: Vec::new(),
                        serial_await_in_loop: Vec::new(),
                        pd_concat_in_loop: Vec::new(),
                        blocking_sync_in_async: Vec::new(),
                        blocking_io_under_lock: Vec::new(),
                        array_spread_in_reduce: Vec::new(),
                        sql_cartesian_join: Vec::new(),
                        defer_in_loop: Vec::new(),
                        goroutine_in_unbounded_loop: Vec::new(),
                        membership_test_in_loop: Vec::new(),
                        sync_io_calls: Vec::new(),
                        param_count,
                        primitive_param_count,
                        body_hash,
                    });
                    self.scope_stack.push(id);
                    self.visit_children(node);
                    self.scope_stack.pop();
                    return;
                }
            }
            "function_call" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    if name_node.kind() == "identifier" && text(name_node, self.source) == "require"
                    {
                        if let Some(spec) = require_argument(node, self.source) {
                            self.imports.push(ImportRef {
                                path: spec,
                                line: self.line_of(node),
                                resolved_file: None,
                            });
                        }
                    } else {
                        let callee_name = call_target_name(name_node, self.source);
                        if !callee_name.is_empty() {
                            self.calls.push(CallRef {
                                caller: self.current_scope(),
                                callee_name,
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

/// For `obj.field()`/`obj:method()` return the last segment (`field`/
/// `method`); for a bare `func()` return the identifier itself. `None` for
/// a call target this shape doesn't cover (a parenthesized or chained
/// call expression, e.g. `(f())()`) -- rare enough in practice that
/// falling back to an empty, filtered-out name matches Go's own
/// `call_target_name` fallback.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "dot_index_expression" => node
            .child_by_field_name("field")
            .map(|f| text(f, source).to_string())
            .unwrap_or_default(),
        "method_index_expression" => node
            .child_by_field_name("method")
            .map(|m| text(m, source).to_string())
            .unwrap_or_default(),
        "identifier" => text(node, source).to_string(),
        _ => String::new(),
    }
}

/// `require(...)`'s first argument, as either a plain string's unquoted
/// content (`require("Bar")`) or -- Roblox's instance-tree convention --
/// the raw source text of a dotted path expression
/// (`require(script.Parent.Foo)` -> `"script.Parent.Foo"`). `None` when
/// the call has no arguments at all.
fn require_argument(call_node: Node, source: &str) -> Option<String> {
    let args_field = call_node.child_by_field_name("arguments")?;
    let arg = if args_field.kind() == "arguments" {
        args_field.named_child(0)?
    } else {
        // Bare call-sugar forms (`require "Bar"`) alias the `arguments`
        // field directly to the single string/table argument rather than
        // wrapping it in an `arguments` node.
        args_field
    };
    if arg.kind() == "string" {
        let content = arg.child_by_field_name("content");
        Some(
            content
                .map(|c| text(c, source).to_string())
                .unwrap_or_default(),
        )
    } else {
        Some(text(arg, source).to_string())
    }
}

/// Cyclomatic-complexity decision points for Luau: `if`/`elseif` (both the
/// statement and ternary-`if`-expression forms), `while`/`repeat`/`for`
/// loops, and short-circuiting boolean operators (`and`/`or`). Luau has no
/// exception-handling construct (`pcall`/`xpcall` are ordinary function
/// calls, not syntax) to add a decision point for, unlike Go's
/// `catch_block`/Kotlin's `catch_block` equivalents.
fn is_decision(n: Node) -> bool {
    match n.kind() {
        "if_statement" | "elseif_statement" | "if_expression" | "elseif_clause"
        | "while_statement" | "repeat_statement" | "for_statement" => true,
        "binary_expression" => is_boolean_operator(n),
        _ => false,
    }
}

/// A short-circuiting `and`/`or` -- `binary_expression` has no `operator`
/// field in this grammar (unlike Go/Kotlin/JS), so the operator token is
/// read positionally: `left`, then the operator, then `right`.
fn is_boolean_operator(n: Node) -> bool {
    n.kind() == "binary_expression" && matches!(n.child(1).map(|c| c.kind()), Some("and" | "or"))
}

/// The condition sub-expression of an `if`/`elseif`/`while`/`repeat`.
fn condition_of(n: Node) -> Option<Node> {
    match n.kind() {
        "if_statement" | "elseif_statement" | "if_expression" | "elseif_clause"
        | "while_statement" | "repeat_statement" => n.child_by_field_name("condition"),
        _ => None,
    }
}

/// Only nested named function declarations get their own symbol; an
/// anonymous `function_definition` closure (`local f = function() ... end`,
/// or one passed inline as a callback) doesn't, so its branches fold into
/// the enclosing scope's count -- same tradeoff already made for Rust's
/// untracked closures.
fn is_nested_function(n: Node) -> bool {
    n.kind() == "function_declaration"
}

/// A `parameter` node's declared type annotation (`x: number` -> the
/// `number` node), if any -- unlike most languages' grammars, this
/// grammar exposes the type as a plain positional second named child
/// rather than a `type` field.
fn param_type(n: Node, source: &str) -> Option<String> {
    if n.kind() != "parameter" || n.named_child_count() < 2 {
        return None;
    }
    n.named_child(1).map(|t| text(t, source).to_string())
}

/// Luau's built-in scalar/primitive type names -- domain types (parsed as
/// a plain `identifier` type node rather than `builtin_type`) never match
/// this table, so they're correctly excluded regardless of spelling.
fn is_primitive_type(t: &str) -> bool {
    matches!(
        t,
        "string" | "number" | "boolean" | "nil" | "thread" | "userdata" | "buffer"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("test.luau"), source).unwrap()
    }

    #[test]
    fn extracts_plain_and_local_function_declarations() {
        let rec = extract_str(
            "local function topLevel(a, b)\n  return helper(a, b)\nend\n\nfunction standalone(x)\n  return x\nend\n",
        );
        let top_level = rec.symbols.iter().find(|s| s.name == "topLevel").unwrap();
        assert_eq!(top_level.kind, SymbolKind::Function);
        assert!(top_level.parent.is_none());
        assert_eq!(top_level.param_count, 2);

        let standalone = rec.symbols.iter().find(|s| s.name == "standalone").unwrap();
        assert_eq!(standalone.kind, SymbolKind::Function);

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "helper");
        assert_eq!(rec.calls[0].caller, Some(top_level.id.clone()));
    }

    #[test]
    fn extracts_dot_and_colon_defined_methods_attributed_to_their_table() {
        let rec = extract_str(
            "local Widget = {}\n\nfunction Widget.new(x)\n  return x\nend\n\nfunction Widget:getX()\n  return self.x\nend\n",
        );
        let new_fn = rec.symbols.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(new_fn.kind, SymbolKind::Method);
        assert_eq!(new_fn.parent.as_deref(), Some("Widget"));

        let get_x = rec.symbols.iter().find(|s| s.name == "getX").unwrap();
        assert_eq!(get_x.kind, SymbolKind::Method);
        assert_eq!(get_x.parent.as_deref(), Some("Widget"));
    }

    #[test]
    fn extracts_require_calls_as_imports_both_string_and_roblox_path_forms() {
        let rec =
            extract_str("local Foo = require(script.Parent.Foo)\nlocal Bar = require(\"Bar\")\n");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"script.Parent.Foo"));
        assert!(paths.contains(&"Bar"));
        assert!(rec.imports.iter().all(|i| i.resolved_file.is_none()));
        // `require` calls are recorded as imports, not calls.
        assert!(!rec.calls.iter().any(|c| c.callee_name == "require"));
    }

    #[test]
    fn records_bare_dotted_and_colon_calls() {
        let rec = extract_str(
            "local Widget = {}\n\nfunction Widget.new()\n  return setmetatable({}, Widget)\nend\n\nfunction use()\n  local w = Widget.new()\n  w:getX()\n  print(w.x)\nend\n",
        );
        let callees: Vec<_> = rec.calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(callees.contains(&"setmetatable"));
        assert!(callees.contains(&"new"));
        assert!(callees.contains(&"getX"));
        assert!(callees.contains(&"print"));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_str(
            "local function straightLine(a, b)\n  return a + b\nend\n\nlocal function branchy(x, y, z)\n  if x > 0 and y > 0 then\n    return 1\n  elseif z > 0 then\n    return 2\n  end\n  for i = 1, x do\n    if i == y then\n      return i\n    end\n  end\n  return 0\nend\n",
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "straightLine")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + and(1) + elseif(1) + for(1) + if(1) = 6
        assert_eq!(branchy.complexity, 6);
        assert_eq!(branchy.param_count, 3);
    }

    #[test]
    fn hashes_duplicate_function_bodies_identically() {
        let rec = extract_str(
            "local function one(n)\n  local total = 0\n  for i = 1, n do\n    total += i\n  end\n  return total\nend\n\nlocal function two(n)\n  local total = 0\n  for i = 1, n do\n    total += i\n  end\n  return total\nend\n\nlocal function short()\n  return 1\nend\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "short").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }

    #[test]
    fn computes_primitive_param_count() {
        let rec = extract_str(
            "local function process(id: number, name: string, active: boolean, cfg: Config)\nend\n",
        );
        let process = rec.symbols.iter().find(|s| s.name == "process").unwrap();
        assert_eq!(process.param_count, 4);
        assert_eq!(process.primitive_param_count, 3);
    }

    #[test]
    fn flags_conditions_chaining_three_or_more_boolean_operators() {
        let rec = extract_str(
            "local function check(a, b, c, d)\n  if a > 0 and b > 0 and c > 0 and d > 0 then\n    print(\"ok\")\n  end\nend\n",
        );
        let check = rec.symbols.iter().find(|s| s.name == "check").unwrap();
        assert_eq!(check.complex_conditionals.len(), 1);
        assert_eq!(check.complex_conditionals[0].operator_count, 3);
    }

    #[test]
    fn field_accesses_are_left_empty_matching_kotlin_and_gos_depth() {
        let rec =
            extract_str("local Widget = {}\n\nfunction Widget:getX()\n  return self.x\nend\n");
        assert!(rec.field_accesses.is_empty());
    }
}
