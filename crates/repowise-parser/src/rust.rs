use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FieldAccessRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::{Path, PathBuf};
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
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
        impl_type_stack: Vec::new(),
    };
    walker.visit(tree.root_node());

    Ok(FileRecord {
        path: path.to_path_buf(),
        language: Language::Rust,
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
    /// Stack of enclosing symbol ids, innermost last.
    scope_stack: Vec<String>,
    /// Stack of enclosing `impl Type` names, innermost last.
    impl_type_stack: Vec<String>,
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
            "function_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let parent = self.impl_type_stack.last().cloned();
                    let kind = if parent.is_some() {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };
                    let body = node.child_by_field_name("body");
                    let complexity = body
                        .map(|b| {
                            metrics::cyclomatic_complexity(
                                b,
                                |n| is_decision(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or(0);
                    let max_nesting_depth = body
                        .map(|b| {
                            metrics::max_nesting_depth(
                                b,
                                |n| is_decision(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or(0);
                    let bumpy_road_bumps = body
                        .map(|b| {
                            metrics::bumpy_road_bumps(
                                b,
                                |n| is_decision(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or(0);
                    let complex_conditionals = body
                        .map(|b| {
                            metrics::complex_conditionals(
                                b,
                                condition_of,
                                |n| is_boolean_operator(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let io_in_loop = body
                        .map(|b| {
                            metrics::calls_in_loops(
                                b,
                                is_loop,
                                |n| call_expression_callee(n, self.source),
                                is_io_call,
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let string_concat_in_loop = body
                        .map(|b| {
                            metrics::string_concats_in_loops(
                                b,
                                is_loop,
                                |n| is_string_concat(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let resource_construction_in_loop = body
                        .map(|b| {
                            metrics::resource_constructions_in_loops(
                                b,
                                is_loop,
                                |n| qualified_call_name(n, self.source),
                                is_expensive_constructor,
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let lock_in_loop = body
                        .map(|b| {
                            metrics::locks_in_loops(
                                b,
                                is_loop,
                                |n| call_expression_callee(n, self.source),
                                is_lock_call,
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let list_insert_zero_in_loop = body
                        .map(|b| {
                            metrics::list_inserts_zero_in_loops(
                                b,
                                is_loop,
                                |n| is_list_insert_zero(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let json_parse_in_loop = body
                        .map(|b| {
                            metrics::json_parses_in_loops(
                                b,
                                is_loop,
                                |n| qualified_call_name(n, self.source),
                                is_json_parse_call,
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let regex_compile_in_loop = body
                        .map(|b| {
                            metrics::regex_compiles_in_loops(
                                b,
                                is_loop,
                                |n| qualified_call_name(n, self.source),
                                is_regex_compile_call,
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let nested_loop_with_io = body
                        .map(|b| {
                            metrics::ios_in_nested_loops(
                                b,
                                is_loop,
                                |n| call_expression_callee(n, self.source),
                                is_io_call,
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let nested_loop_quadratic = body
                        .map(|b| {
                            metrics::quadratic_loop_nestings(
                                b,
                                |n| loop_iterable(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let serial_await_in_loop = body
                        .map(|b| {
                            metrics::serial_awaits_in_loops(
                                b,
                                is_loop,
                                |n| awaited_callee(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let blocking_sync_in_async = if is_async_fn(node, self.source) {
                        body.map(|b| {
                            metrics::blocking_calls_in_async(
                                b,
                                |n| blocking_callee(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let blocking_io_under_lock = body
                        .map(|b| {
                            metrics::ios_under_lock_binding(
                                b,
                                |n| is_lock_binding(n, self.source),
                                |n| call_expression_callee(n, self.source),
                                is_io_call,
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let sql_cartesian_join = body
                        .map(|b| {
                            metrics::sql_cartesian_joins(
                                b,
                                |n| string_literal_content(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let membership_test_in_loop = body
                        .map(|b| {
                            metrics::list_membership_tests_in_loops(
                                b,
                                is_loop,
                                |n| collection_binding(n, self.source),
                                |n| membership_target(n, self.source),
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let sync_io_calls = body
                        .map(|b| {
                            metrics::sync_io_calls_in_body(
                                b,
                                |n| call_expression_callee(n, self.source),
                                is_io_call,
                                |n| n.kind() == "function_item",
                            )
                        })
                        .unwrap_or_default();
                    let param_count = metrics::count_params(node.child_by_field_name("parameters"));
                    let primitive_param_count = metrics::primitive_param_count(
                        node.child_by_field_name("parameters"),
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
                        param_count,
                        primitive_param_count,
                        body_hash,
                        io_in_loop,
                        string_concat_in_loop,
                        resource_construction_in_loop,
                        lock_in_loop,
                        list_insert_zero_in_loop,
                        json_parse_in_loop,
                        regex_compile_in_loop,
                        nested_loop_with_io,
                        nested_loop_quadratic,
                        serial_await_in_loop,
                        // `pd_concat_in_loop` is Python-only (issue #192):
                        // pandas is a Python library with no equivalent in
                        // this port's other supported languages.
                        pd_concat_in_loop: Vec::new(),
                        blocking_sync_in_async,
                        blocking_io_under_lock,
                        // `array_spread_in_reduce` is TypeScript/JavaScript-only
                        // (issue #194): it targets the JS array method, which has no
                        // equivalent in this port's other languages.
                        array_spread_in_reduce: Vec::new(),
                        sql_cartesian_join,
                        defer_in_loop: Vec::new(),
                        goroutine_in_unbounded_loop: Vec::new(),
                        membership_test_in_loop,
                        sync_io_calls,
                    });
                    self.scope_stack.push(id);
                    self.visit_children(node);
                    self.scope_stack.pop();
                    return;
                }
            }
            "struct_item" | "enum_item" | "trait_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let kind = match node.kind() {
                        "struct_item" => SymbolKind::Struct,
                        "enum_item" => SymbolKind::Enum,
                        _ => SymbolKind::Trait,
                    };
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name,
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
                    });
                }
            }
            "mod_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    self.symbols.push(Symbol {
                        id: Symbol::make_id(self.path, &name, start_line),
                        name: name.clone(),
                        kind: SymbolKind::Module,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line: node.end_position().row + 1,
                        parent: None,
                        complexity: 0,
                        max_nesting_depth: 0,
                        bumpy_road_bumps: 0,
                        complex_conditionals: Vec::new(),
                        param_count: 0,
                        primitive_param_count: 0,
                        body_hash: None,
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
                    });
                    // `mod foo;` (no inline body) declares that another
                    // file defines this module. Resolve it directly via
                    // Rust's file-layout convention rather than relying on
                    // the module-index heuristic in repowise-graph.
                    if node.child_by_field_name("body").is_none() {
                        if let Some(target) = resolve_mod_file(self.path, &name) {
                            self.imports.push(ImportRef {
                                path: format!("mod {name}"),
                                line: start_line,
                                resolved_file: Some(target),
                            });
                        }
                    }
                }
            }
            "impl_item" => {
                if let Some(type_node) = node.child_by_field_name("type") {
                    let type_name = last_path_segment(text(type_node, self.source));
                    self.impl_type_stack.push(type_name);
                    self.visit_children(node);
                    self.impl_type_stack.pop();
                    return;
                }
            }
            "use_declaration" => {
                if let Some(arg) = node.child_by_field_name("argument") {
                    let mut paths = Vec::new();
                    flatten_use(arg, "", self.source, &mut paths);
                    let line = self.line_of(node);
                    for p in paths {
                        self.imports.push(ImportRef {
                            path: p,
                            line,
                            resolved_file: None,
                        });
                    }
                }
            }
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    let callee_name = call_target_name(func, self.source);
                    self.calls.push(CallRef {
                        caller: self.current_scope(),
                        callee_name,
                        line: self.line_of(node),
                    });
                }
            }
            "field_expression" => {
                if let (Some(value), Some(field)) = (
                    node.child_by_field_name("value"),
                    node.child_by_field_name("field"),
                ) {
                    if text(value, self.source) == "self" && !is_call_target(node) {
                        if let Some(method) = self.current_scope() {
                            self.field_accesses.push(FieldAccessRef {
                                method,
                                field_name: text(field, self.source).to_string(),
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

/// For a call target expression, return the name that should be matched
/// against known symbol names: the identifier itself, the field name for
/// `receiver.method()`, or the last segment of a `path::to::func()`.
fn call_target_name(node: Node, source: &str) -> String {
    match node.kind() {
        "identifier" => text(node, source).to_string(),
        "field_expression" => node
            .child_by_field_name("field")
            .map(|f| text(f, source).to_string())
            .unwrap_or_else(|| text(node, source).to_string()),
        "scoped_identifier" => node
            .child_by_field_name("name")
            .map(|n| text(n, source).to_string())
            .unwrap_or_else(|| last_path_segment(text(node, source))),
        _ => last_path_segment(text(node, source)),
    }
}

fn last_path_segment(s: &str) -> String {
    s.rsplit("::").next().unwrap_or(s).to_string()
}

/// True when `node` (a `field_expression`) is the `function` position of
/// its parent `call_expression` — i.e. `self.method()` rather than a
/// field read/write like `self.field`. Excluded from field-access
/// tracking so method names don't pollute the field-cohesion signal.
fn is_call_target(node: Node) -> bool {
    node.parent()
        .map(|p| {
            p.kind() == "call_expression"
                && p.child_by_field_name("function").map(|f| f.id()) == Some(node.id())
        })
        .unwrap_or(false)
}

/// Cyclomatic-complexity decision points for Rust: branches, loops, match
/// arms, and short-circuiting boolean operators (`&&` / `||`).
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_expression"
        | "if_let_expression"
        | "match_arm"
        | "while_expression"
        | "while_let_expression"
        | "loop_expression"
        | "for_expression" => true,
        "binary_expression" => is_boolean_operator(n, source),
        _ => false,
    }
}

/// A short-circuiting boolean operator (`&&` / `||`) -- a separate
/// helper from `is_decision` since `complex_conditionals` counts these
/// within one condition's own subtree, not decision points across the
/// whole function body.
fn is_boolean_operator(n: Node, source: &str) -> bool {
    n.kind() == "binary_expression"
        && n.child_by_field_name("operator")
            .map(|op| matches!(text(op, source), "&&" | "||"))
            .unwrap_or(false)
}

/// The condition sub-expression of an `if`/`while` (plain, not
/// `if let`/`while let`, whose condition is a pattern-match rather than
/// a boolean expression and isn't in scope for this marker).
fn condition_of(n: Node) -> Option<Node> {
    match n.kind() {
        "if_expression" | "while_expression" => n.child_by_field_name("condition"),
        _ => None,
    }
}

/// Loop constructs for `io_in_loop` (issue #177): every way Rust repeats
/// a block. A subset of `is_decision`'s node kinds -- excludes
/// `if_expression`/`if_let_expression`/`match_arm`, which branch but
/// don't repeat.
fn is_loop(n: Node) -> bool {
    matches!(
        n.kind(),
        "while_expression" | "while_let_expression" | "loop_expression" | "for_expression"
    )
}

/// Collection kind for a Rust type/constructor base name, for
/// `membership_test_in_loop` (issue #182). This distinction matters more
/// in Rust than anywhere else in the marker: `Vec::contains` and
/// `HashSet::contains` are spelled identically at the call site, so the
/// binding is the *only* thing that separates an O(n) scan from an O(1)
/// lookup.
fn collection_kind_for_type(name: &str) -> Option<metrics::CollectionKind> {
    match name {
        "Vec" | "VecDeque" => Some(metrics::CollectionKind::List),
        "HashSet" | "BTreeSet" | "HashMap" | "BTreeMap" => Some(metrics::CollectionKind::NotList),
        _ => None,
    }
}

/// A `let name: Ty = init;` binding whose declared type or initializer
/// shape settles the collection's kind, for `membership_test_in_loop`
/// (issue #182). The declared type wins when present -- it's the more
/// direct statement of intent, and it's what makes
/// `let seen: HashSet<_> = xs.into_iter().collect();` resolvable when
/// the initializer alone (`.collect()`) says nothing.
fn collection_binding(n: Node, source: &str) -> Option<(String, metrics::CollectionKind)> {
    if n.kind() != "let_declaration" {
        return None;
    }
    let pattern = n.child_by_field_name("pattern")?;
    if pattern.kind() != "identifier" {
        return None;
    }
    let name = text(pattern, source).to_string();

    if let Some(ty) = n.child_by_field_name("type") {
        let base = match ty.kind() {
            "generic_type" => ty.child_by_field_name("type")?,
            "type_identifier" => ty,
            _ => return None,
        };
        return collection_kind_for_type(text(base, source)).map(|kind| (name, kind));
    }

    let value = n.child_by_field_name("value")?;
    let kind = match value.kind() {
        "array_expression" => metrics::CollectionKind::List,
        // `vec![..]`
        "macro_invocation" => {
            let macro_name = value.child_by_field_name("macro")?;
            if text(macro_name, source) == "vec" {
                metrics::CollectionKind::List
            } else {
                return None;
            }
        }
        // `Vec::new()` / `HashSet::from(..)`: the path's first segment
        // is the type.
        "call_expression" => {
            let func = value.child_by_field_name("function")?;
            if func.kind() != "scoped_identifier" {
                return None;
            }
            let base = func.child_by_field_name("path")?;
            collection_kind_for_type(text(base, source))?
        }
        _ => return None,
    };
    Some((name, kind))
}

/// An `xs.contains(&x)` call, for `membership_test_in_loop` (issue
/// #182). Returns what's being tested against; the caller decides
/// whether that's a list. `slice::contains`/`Vec::contains` are O(n)
/// while `HashSet::contains` is O(1), and nothing at this call site
/// tells them apart -- that's what the binding map is for.
fn membership_target(n: Node, source: &str) -> Option<metrics::MembershipTarget> {
    if n.kind() != "call_expression" {
        return None;
    }
    let func = n.child_by_field_name("function")?;
    if func.kind() != "field_expression" {
        return None;
    }
    let field = func.child_by_field_name("field")?;
    if text(field, source) != "contains" {
        return None;
    }
    let receiver = func.child_by_field_name("value")?;
    match receiver.kind() {
        "array_expression" => Some(metrics::MembershipTarget::InlineList),
        "identifier" => Some(metrics::MembershipTarget::Named(
            text(receiver, source).to_string(),
        )),
        _ => None,
    }
}

/// True when `node` is a `let` binding whose initializer acquires a
/// lock, for `blocking_io_under_lock` (issue #185). Such a binding holds
/// the guard until the end of its enclosing block, so everything after
/// it in that block runs inside the critical section.
///
/// Reuses `is_lock_call`'s table, so it inherits that table's
/// deliberate exclusion of `RwLock::read`/`write` -- those bare method
/// names are too generic to match safely without type information.
/// Coarse in the other direction too: `let _ = m.lock();` actually drops
/// the guard immediately, but is treated as a binding here.
fn is_lock_binding(node: Node, source: &str) -> bool {
    if node.kind() != "let_declaration" {
        return false;
    }
    let Some(value) = node.child_by_field_name("value") else {
        return false;
    };
    contains_lock_call(value, source)
}

/// Whether any call in this subtree acquires a lock -- covers the
/// `m.lock().unwrap()` shape where the acquisition is nested under the
/// `unwrap`.
fn contains_lock_call(node: Node, source: &str) -> bool {
    if call_expression_callee(node, source).is_some_and(|n| is_lock_call(&n)) {
        return true;
    }
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|c| contains_lock_call(c, source));
    found
}

/// The text inside a string-literal node, for `sql_cartesian_join`
/// (issue #195). Covers both plain and raw string literals. Returns `None` for any other node.
fn string_literal_content(node: Node, source: &str) -> Option<String> {
    if !matches!(node.kind(), "string_literal" | "raw_string_literal") {
        return None;
    }
    let mut cursor = node.walk();
    let content = node
        .named_children(&mut cursor)
        .find(|c| matches!(c.kind(), "string_content" | "string_fragment"));
    content.map(|c| text(c, source).to_string())
}

/// True when this `function_item` is declared `async fn`, for
/// `blocking_sync_in_async` (issue #184). Unlike every loop-body marker,
/// this marker's context is the enclosing *function*, so the check
/// happens on the function node itself rather than during a body walk.
fn is_async_fn(node: Node, source: &str) -> bool {
    if node.kind() != "function_item" {
        return false;
    }
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .any(|c| c.kind() == "function_modifiers" && text(c, source).contains("async"));
    found
}

/// The callee of a blocking synchronous call, for
/// `blocking_sync_in_async` (issue #184): `thread::sleep` for
/// `std::thread::sleep(d)`. Matched on the qualified two-segment path,
/// since a bare `sleep`/`read`/`write` would be far too generic.
///
/// A call that is itself being `.await`ed is never reported, which is
/// what keeps `tokio::fs::read_to_string(p).await` from being mistaken
/// for `std::fs::read_to_string(p)`: both reduce to the same
/// `fs::read_to_string` two-segment path, and being awaited is the only
/// local evidence distinguishing the async variant from the blocking
/// one.
fn blocking_callee(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    if node
        .parent()
        .is_some_and(|p| p.kind() == "await_expression")
    {
        return None;
    }
    let name = qualified_call_name(node, source)?;
    is_blocking_call(&name).then_some(name)
}

/// A small fixed table of blocking `std` paths -- heuristic and coarse,
/// like `is_io_call`. Deliberately limited to calls with a clear async
/// replacement, so every hit has an actionable fix.
fn is_blocking_call(name: &str) -> bool {
    matches!(
        name,
        "thread::sleep"
            | "fs::read_to_string"
            | "fs::read"
            | "fs::write"
            | "fs::copy"
            | "fs::remove_file"
            | "fs::create_dir_all"
    )
}

/// The callee of an awaited async call, for `serial_await_in_loop`
/// (issue #181): `fetch` for `fetch(u).await`. `None` for a non-await
/// node, for an await whose operand isn't a call at all (`x.await` on an
/// already-created future -- rarer, and not the "each iteration's async
/// call" shape the issue describes), and for awaits of the concurrency
/// combinators that *are* the fix -- awaiting a `join_all` inside a loop
/// is chunked concurrency, not the serial pattern this flags.
fn awaited_callee(node: Node, source: &str) -> Option<String> {
    if node.kind() != "await_expression" {
        return None;
    }
    let awaited = node.named_child(0)?;
    if awaited.kind() != "call_expression" {
        return None;
    }
    let name = call_target_name(awaited.child_by_field_name("function")?, source);
    if is_concurrency_combinator(&name) {
        return None;
    }
    Some(name)
}

/// Futures-combinator names that batch a whole set of awaits into one
/// concurrent wait -- i.e. the fix `serial_await_in_loop` points at, so
/// awaiting one is never itself the problem.
fn is_concurrency_combinator(name: &str) -> bool {
    matches!(
        name,
        "join_all" | "try_join_all" | "join" | "try_join" | "select_all"
    )
}

/// The base collection a `for` loop iterates over, for
/// `nested_loop_quadratic` (issue #187): `items` for all of
/// `for x in items`, `for x in &items`, and `for x in items.iter()`.
/// `None` for `while`/`loop` (no iterable to compare) and for any
/// iterable that doesn't normalize down to a plain identifier.
///
/// Ranges (`for i in 0..n`) are deliberately excluded even though a
/// doubly-nested one is also quadratic: that shape is usually a
/// deliberate, irreducible grid/matrix traversal, whereas iterating the
/// same *collection* twice is the accidental all-pairs scan this marker
/// is after -- the one that's usually replaceable with a set/map lookup.
fn loop_iterable(node: Node, source: &str) -> Option<String> {
    if node.kind() != "for_expression" {
        return None;
    }
    base_collection_name(node.child_by_field_name("value")?, source)
}

/// Peel `&`/`&mut` and a trailing iterator-adapter call off an iterable
/// expression, returning the underlying identifier if one is left. Only
/// adapters that yield the *same* underlying collection are peeled, so
/// `items` and `items.iter()` compare equal while `items.filter(..)`
/// (a different, narrower sequence) doesn't normalize at all.
fn base_collection_name(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" => Some(text(node, source).to_string()),
        "reference_expression" => base_collection_name(
            node.child_by_field_name("value")
                .or_else(|| node.named_child(0))?,
            source,
        ),
        "call_expression" => {
            let func = node.child_by_field_name("function")?;
            if func.kind() != "field_expression" {
                return None;
            }
            let field = func.child_by_field_name("field")?;
            if !matches!(
                text(field, source),
                "iter" | "iter_mut" | "into_iter" | "clone" | "as_slice" | "values" | "keys"
            ) {
                return None;
            }
            base_collection_name(func.child_by_field_name("value")?, source)
        }
        _ => None,
    }
}

/// If `node` is a `call_expression`, the callee name to match against
/// `is_io_call` -- same extraction `call_target_name` already does for
/// the file's general call graph, applied here to a single node rather
/// than every call in the file.
fn call_expression_callee(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    node.child_by_field_name("function")
        .map(|f| call_target_name(f, source))
}

/// A small fixed table of I/O-shaped callee names (file, network, or
/// database operations) -- heuristic and coarse, like
/// `repowise_workspace::contracts`'s route-pattern table: it matches on
/// the same last-path-segment name `call_target_name` already uses for
/// the general call graph, so it can't tell `db.execute(..)` from an
/// unrelated `execute` method on some other type, and it can't recognize
/// I/O hidden behind a wrapper function this table doesn't name.
fn is_io_call(name: &str) -> bool {
    matches!(
        name,
        "read_to_string"
            | "read_to_end"
            | "read_line"
            | "read_exact"
            | "write_all"
            | "execute"
            | "query"
            | "query_row"
            | "fetch_one"
            | "fetch_all"
            | "recv"
    )
}

/// A string-append expression for `string_concat_in_loop` (issue #178):
/// `s += other` (`compound_assignment_expr`), `s = s + other`
/// (`assignment_expression` whose right side is a `+` `binary_expression`
/// naming `s` on one side), or `s.push_str(other)` (a `call_expression`
/// whose callee is a `push_str` method on a bare identifier). Returns the
/// appended-onto variable's name.
fn is_string_concat(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "compound_assignment_expr" => {
            let left = node.child_by_field_name("left")?;
            let operator = node.child_by_field_name("operator")?;
            if left.kind() == "identifier" && text(operator, source) == "+=" {
                Some(text(left, source).to_string())
            } else {
                None
            }
        }
        "assignment_expression" => {
            let left = node.child_by_field_name("left")?;
            if left.kind() != "identifier" {
                return None;
            }
            let left_name = text(left, source);
            let right = node.child_by_field_name("right")?;
            if right.kind() != "binary_expression"
                || right
                    .child_by_field_name("operator")
                    .map(|op| text(op, source))
                    != Some("+")
            {
                return None;
            }
            let bin_left = right.child_by_field_name("left")?;
            let bin_right = right.child_by_field_name("right")?;
            if text(bin_left, source) == left_name || text(bin_right, source) == left_name {
                Some(left_name.to_string())
            } else {
                None
            }
        }
        "call_expression" => {
            let func = node.child_by_field_name("function")?;
            if func.kind() != "field_expression" {
                return None;
            }
            let field = func.child_by_field_name("field")?;
            if text(field, source) != "push_str" {
                return None;
            }
            let value = func.child_by_field_name("value")?;
            (value.kind() == "identifier").then(|| text(value, source).to_string())
        }
        _ => None,
    }
}

/// For a call node whose target is a fully-qualified `Type::method` path
/// (a `scoped_identifier`, e.g. `reqwest::HttpClient::new()`), the last
/// two path segments joined by `::` (e.g. `HttpClient::new`). Used for
/// `resource_construction_in_loop` (issue #179) instead of
/// `call_target_name`'s plain last-segment match, because a bare method
/// name like `new` is far too generic on its own -- `Vec::new()`/
/// `String::new()` are cheap and must not match. A method call on a
/// receiver *value* (`field_expression`, e.g. `pool.get_connection()`)
/// returns `None`: this port has no type information to know what type
/// owns the receiver, so those are left unrecognized rather than guessed.
fn qualified_call_name(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    if func.kind() != "scoped_identifier" {
        return None;
    }
    let method = func.child_by_field_name("name")?;
    let path = func.child_by_field_name("path")?;
    let type_name = last_path_segment(text(path, source));
    Some(format!("{type_name}::{}", text(method, source)))
}

/// A small fixed table of `Type::method` constructor paths recognized as
/// building an expensive resource (an HTTP client, a connection/thread
/// pool) -- heuristic and coarse, like `is_io_call`: it can't recognize
/// an expensive constructor hidden behind a type alias or a wrapper
/// function this table doesn't name. Deliberately excludes regex
/// construction (`Regex::new`) -- reserved for `regex_compile_in_loop`
/// (issue #188) so the two markers don't double-flag the same call once
/// both exist -- and excludes plain-allocation constructors
/// (`Vec::with_capacity`, `String::new`, etc.), which the issue's own
/// acceptance criteria names as a required non-match.
fn is_expensive_constructor(name: &str) -> bool {
    matches!(
        name,
        "HttpClient::new" | "Client::new" | "ThreadPool::new" | "ConnectionPool::new" | "Pool::new"
    )
}

/// A small fixed table of lock-acquisition method names for
/// `lock_in_loop` (issue #180): `Mutex`'s `.lock()`/`.try_lock()`.
/// Deliberately excludes `RwLock::read`/`RwLock::write`, which also
/// acquire a lock: those bare method names are far too generic on their
/// own (the `Read`/`Write` trait methods, plain field getters/setters,
/// etc. share the same names), and this port has no type information to
/// know a given receiver is actually an `RwLock`.
fn is_lock_call(name: &str) -> bool {
    matches!(name, "lock" | "try_lock")
}

/// A small fixed table of `module::function` paths recognized as parsing
/// a JSON payload for `json_parse_in_loop` (issue #193) -- heuristic and
/// coarse, like `is_io_call`: it can't recognize a JSON-parsing call
/// hidden behind a wrapper function or an alias this table doesn't name.
fn is_json_parse_call(name: &str) -> bool {
    matches!(name, "serde_json::from_str" | "serde_json::from_slice")
}

/// A small fixed table of `Type::method` paths recognized as compiling a
/// regex for `regex_compile_in_loop` (issue #188) -- heuristic and
/// coarse, like `is_io_call`. This is exactly the pattern
/// `is_expensive_constructor`'s doc comment named as reserved for this
/// marker.
fn is_regex_compile_call(name: &str) -> bool {
    matches!(name, "Regex::new")
}

/// A `.insert(0, ...)` call on a bare identifier for
/// `list_insert_zero_in_loop` (issue #191): a `call_expression` whose
/// callee is an `insert` method on an identifier receiver (covers both
/// `Vec::insert`/`VecDeque::insert`, since this port has no type
/// information to distinguish the two collection types), whose first
/// argument is the literal `0`. Returns the receiver's variable name.
/// Unlike `is_io_call`/`is_lock_call`'s plain name-table shape, this
/// needs to inspect the call's arguments too, so it's a single combined
/// classifier rather than a name lookup.
fn is_list_insert_zero(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    if func.kind() != "field_expression" {
        return None;
    }
    let field = func.child_by_field_name("field")?;
    if text(field, source) != "insert" {
        return None;
    }
    let value = func.child_by_field_name("value")?;
    if value.kind() != "identifier" {
        return None;
    }
    let arguments = node.child_by_field_name("arguments")?;
    let first_arg = arguments.named_child(0)?;
    if first_arg.kind() == "integer_literal" && text(first_arg, source) == "0" {
        Some(text(value, source).to_string())
    } else {
        None
    }
}

/// A parameter's declared type as source text, with a leading `&`/`&mut`/
/// lifetime reference prefix stripped so `&str`/`&'a String` classify the
/// same as their owned form. Only plain `parameter` nodes carry a `type`
/// field — `self_parameter`/`variadic_parameter` don't, so `self` and `...`
/// are naturally excluded rather than double-handled here.
fn param_type(n: Node, source: &str) -> Option<String> {
    if n.kind() != "parameter" {
        return None;
    }
    let type_node = n.child_by_field_name("type")?;
    Some(strip_reference(text(type_node, source)).to_string())
}

fn strip_reference(mut s: &str) -> &str {
    s = s.trim();
    while let Some(rest) = s.strip_prefix('&') {
        s = rest.trim_start();
        if s.starts_with('\'') {
            if let Some(idx) = s.find(char::is_whitespace) {
                s = s[idx..].trim_start();
            }
        }
        if let Some(rest) = s.strip_prefix("mut ") {
            s = rest.trim_start();
        }
    }
    s
}

/// The bare scalar/string primitives "primitive obsession" flags — the
/// classic smell targets overused strings/ints/bools, so `String`/`str`
/// are included alongside the scalar keyword types even though `String`
/// isn't a `Copy` primitive in Rust's own type system.
fn is_primitive_type(t: &str) -> bool {
    matches!(
        t,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
            | "String"
    )
}

/// Resolve `mod name;` to the file it declares, per Rust's file-layout
/// convention: siblings of `lib.rs`/`main.rs`/`mod.rs` live directly next
/// to it; siblings of any other `foo.rs` live under a `foo/` directory.
fn resolve_mod_file(current_file: &Path, name: &str) -> Option<PathBuf> {
    let dir = current_file.parent()?;
    let stem = current_file.file_stem()?.to_str()?;
    let base_dir = if matches!(stem, "lib" | "main" | "mod") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };

    let as_file = base_dir.join(format!("{name}.rs"));
    if as_file.is_file() {
        return Some(as_file);
    }
    let as_mod_dir = base_dir.join(name).join("mod.rs");
    if as_mod_dir.is_file() {
        return Some(as_mod_dir);
    }
    None
}

/// Recursively flatten a `use` tree node into fully dotted (`::`-joined)
/// import paths, handling grouped (`{a, b}`), aliased (`as`), and wildcard
/// (`*`) imports.
fn flatten_use(node: Node, prefix: &str, source: &str, out: &mut Vec<String>) {
    let join = |prefix: &str, seg: &str| {
        if prefix.is_empty() {
            seg.to_string()
        } else {
            format!("{prefix}::{seg}")
        }
    };
    match node.kind() {
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                flatten_use(child, prefix, source, out);
            }
        }
        "scoped_use_list" => {
            let path_part = node
                .child_by_field_name("path")
                .map(|p| path_text(p, source))
                .unwrap_or_default();
            let new_prefix = join(prefix, &path_part);
            if let Some(list) = node.child_by_field_name("list") {
                flatten_use(list, &new_prefix, source, out);
            }
        }
        "use_as_clause" => {
            if let Some(path) = node.child_by_field_name("path") {
                out.push(join(prefix, &path_text(path, source)));
            }
        }
        "use_wildcard" => {
            let path_part = node
                .child_by_field_name("path")
                .map(|p| path_text(p, source))
                .unwrap_or_default();
            out.push(format!("{}::*", join(prefix, &path_part)));
        }
        _ => {
            // identifier / scoped_identifier / self / super / crate
            out.push(join(prefix, &path_text(node, source)));
        }
    }
}

/// Convert a plain path node (identifier / scoped_identifier / self /
/// super / crate) into a single `::`-joined string.
fn path_text(node: Node, source: &str) -> String {
    match node.kind() {
        "scoped_identifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, source).to_string())
                .unwrap_or_default();
            if let Some(path) = node.child_by_field_name("path") {
                format!("{}::{}", path_text(path, source), name)
            } else {
                name
            }
        }
        _ => text(node, source).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("test.rs"), source).unwrap()
    }

    #[test]
    fn extracts_function_struct_and_method() {
        let rec = extract_str(
            r#"
            struct Foo;

            impl Foo {
                fn bar(&self) -> i32 {
                    baz()
                }
            }

            fn baz() -> i32 { 42 }
            "#,
        );
        let names: Vec<_> = rec.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"baz"));

        let bar = rec.symbols.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Method);
        assert_eq!(bar.parent.as_deref(), Some("Foo"));

        let baz = rec.symbols.iter().find(|s| s.name == "baz").unwrap();
        assert_eq!(baz.kind, SymbolKind::Function);

        assert_eq!(rec.calls.len(), 1);
        assert_eq!(rec.calls[0].callee_name, "baz");
        assert_eq!(rec.calls[0].caller, Some(bar.id.clone()));
    }

    #[test]
    fn records_self_field_reads_and_writes_but_not_method_calls() {
        let rec = extract_str(
            r#"
            struct Point { x: i32, y: i32 }

            impl Point {
                fn shift(&mut self, dx: i32) -> i32 {
                    self.x += dx;
                    self.helper();
                    self.y
                }

                fn helper(&self) {}
            }
            "#,
        );
        let shift = rec.symbols.iter().find(|s| s.name == "shift").unwrap();
        let field_names: Vec<&str> = rec
            .field_accesses
            .iter()
            .filter(|f| f.method == shift.id)
            .map(|f| f.field_name.as_str())
            .collect();
        assert_eq!(field_names, vec!["x", "y"]);
        // `self.helper()` is a method call, not a field access.
        assert!(!field_names.contains(&"helper"));
    }

    #[test]
    fn flattens_grouped_use_declarations() {
        let rec = extract_str("use crate::graph::{build_graph, RepoGraph as Graph};");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"crate::graph::build_graph"));
        assert!(paths.contains(&"crate::graph::RepoGraph"));
    }

    #[test]
    fn computes_cyclomatic_complexity_and_param_count() {
        let rec = extract_str(
            r#"
            fn straight_line(a: i32, b: i32) -> i32 {
                a + b
            }

            fn branchy(x: i32, y: i32, z: i32) -> i32 {
                if x > 0 && y > 0 {
                    return 1;
                } else if z > 0 {
                    return 2;
                }
                for i in 0..x {
                    if i == y {
                        return i;
                    }
                }
                0
            }
            "#,
        );
        let straight = rec
            .symbols
            .iter()
            .find(|s| s.name == "straight_line")
            .unwrap();
        assert_eq!(straight.complexity, 1);
        assert_eq!(straight.param_count, 2);

        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + &&(1) + if-let-else-if(1) + for(1) + if(1) = 6
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
            r#"
            fn sequential(x: i32) -> i32 {
                if x == 1 {
                    return 1;
                }
                if x == 2 {
                    return 2;
                }
                if x == 3 {
                    return 3;
                }
                0
            }

            fn nested(x: i32) -> i32 {
                if x > 0 {
                    if x > 10 {
                        if x > 100 {
                            return 3;
                        }
                        return 2;
                    }
                    return 1;
                }
                0
            }
            "#,
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
            r#"
            fn scattered(x: i32, y: i32, z: i32) -> i32 {
                if x > 0 {
                    if x > 10 {
                        return 1;
                    }
                }
                if y > 0 {
                    if y > 10 {
                        return 2;
                    }
                }
                if z > 0 {
                    if z > 10 {
                        return 3;
                    }
                }
                0
            }

            fn single_deep(x: i32) -> i32 {
                if x > 0 {
                    if x > 10 {
                        return 1;
                    }
                }
                0
            }
            "#,
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
            r#"
            fn tangled(a: bool, b: bool, c: bool, d: bool) -> i32 {
                if a && b && c && d {
                    return 1;
                }
                0
            }

            fn simple(a: bool, b: bool) -> i32 {
                if a && b {
                    return 1;
                }
                0
            }
            "#,
        );
        let tangled = rec.symbols.iter().find(|s| s.name == "tangled").unwrap();
        let simple = rec.symbols.iter().find(|s| s.name == "simple").unwrap();

        assert_eq!(tangled.complex_conditionals.len(), 1);
        assert_eq!(tangled.complex_conditionals[0].operator_count, 3);
        assert!(simple.complex_conditionals.is_empty());
    }

    #[test]
    fn counts_bare_primitive_typed_parameters_but_not_domain_or_reference_types() {
        let rec = extract_str(
            r#"
            struct UserId(u64);

            fn obsessed(name: String, age: u32, active: bool, note: &str) -> bool {
                active
            }

            fn domain_typed(id: UserId, name: &str) -> bool {
                true
            }
            "#,
        );
        let obsessed = rec.symbols.iter().find(|s| s.name == "obsessed").unwrap();
        // String, u32, bool, and &str (reference stripped) are all bare
        // primitives -- all 4 declared parameters count.
        assert_eq!(obsessed.primitive_param_count, 4);

        let domain_typed = rec
            .symbols
            .iter()
            .find(|s| s.name == "domain_typed")
            .unwrap();
        // UserId is a domain type, not a primitive -- only &str counts.
        assert_eq!(domain_typed.primitive_param_count, 1);
    }

    #[test]
    fn hashes_duplicate_function_bodies_identically() {
        let rec = extract_str(
            r#"
            fn one(n: i32) -> i32 {
                let mut total = 0;
                for i in 0..n {
                    total += i;
                }
                total
            }

            fn two(n: i32) -> i32 {
                let mut total = 0;
                for i in 0..n {
                    total += i;
                }
                total
            }

            fn short() -> i32 { 1 }
            "#,
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "short").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        // Too short to be a meaningful duplicate signal.
        assert!(short.body_hash.is_none());
    }

    #[test]
    fn flags_io_shaped_calls_found_inside_a_loop_body_but_not_outside_one() {
        let rec = extract_str(
            r#"
            fn hoisted(paths: Vec<String>) -> Vec<String> {
                let mut out = Vec::new();
                for p in &paths {
                    out.push(std::fs::read_to_string(p).unwrap());
                }
                out
            }

            fn fine(items: Vec<i32>) -> i32 {
                let mut total = 0;
                for i in &items {
                    total += i;
                }
                std::fs::read_to_string("config.toml").ok();
                total
            }
            "#,
        );
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();
        let fine = rec.symbols.iter().find(|s| s.name == "fine").unwrap();

        assert_eq!(hoisted.io_in_loop.len(), 1);
        assert_eq!(hoisted.io_in_loop[0].callee_name, "read_to_string");
        // Same callee, but called after the loop rather than inside it.
        assert!(fine.io_in_loop.is_empty());
    }

    #[test]
    fn flags_string_concat_shapes_found_inside_a_loop_body_but_not_outside_one() {
        let rec = extract_str(
            r#"
            fn compound_assign(items: &[&str]) -> String {
                let mut s = String::new();
                for i in items {
                    s += i;
                }
                s
            }

            fn reassignment(items: &[&str]) -> String {
                let mut s = String::new();
                for i in items {
                    s = s + i;
                }
                s
            }

            fn push_str_call(items: &[&str]) -> String {
                let mut s = String::new();
                for i in items {
                    s.push_str(i);
                }
                s
            }

            fn fine(items: &[&str]) -> String {
                let mut s = String::new();
                for i in items {
                    s.push_str(i);
                }
                s += " done";
                s
            }
            "#,
        );
        let compound_assign = rec
            .symbols
            .iter()
            .find(|s| s.name == "compound_assign")
            .unwrap();
        let reassignment = rec
            .symbols
            .iter()
            .find(|s| s.name == "reassignment")
            .unwrap();
        let push_str_call = rec
            .symbols
            .iter()
            .find(|s| s.name == "push_str_call")
            .unwrap();
        let fine = rec.symbols.iter().find(|s| s.name == "fine").unwrap();

        assert_eq!(compound_assign.string_concat_in_loop.len(), 1);
        assert_eq!(compound_assign.string_concat_in_loop[0].variable, "s");
        assert_eq!(reassignment.string_concat_in_loop.len(), 1);
        assert_eq!(reassignment.string_concat_in_loop[0].variable, "s");
        assert_eq!(push_str_call.string_concat_in_loop.len(), 1);
        assert_eq!(push_str_call.string_concat_in_loop[0].variable, "s");
        // The append inside the loop is flagged, but the one after the
        // loop is not -- "fine" should have exactly the in-loop one.
        assert_eq!(fine.string_concat_in_loop.len(), 1);
    }

    #[test]
    fn flags_expensive_constructors_in_a_loop_but_not_cheap_ones() {
        let rec = extract_str(
            r#"
            fn hoisted(urls: &[&str]) {
                for _u in urls {
                    let client = HttpClient::new();
                    drop(client);
                }
            }

            fn cheap_allocs(items: &[i32]) -> Vec<i32> {
                let mut out = Vec::new();
                for i in items {
                    let buf = Vec::with_capacity(4);
                    out.push(*i);
                    drop(buf);
                }
                out
            }
            "#,
        );
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();
        let cheap_allocs = rec
            .symbols
            .iter()
            .find(|s| s.name == "cheap_allocs")
            .unwrap();

        assert_eq!(hoisted.resource_construction_in_loop.len(), 1);
        assert_eq!(
            hoisted.resource_construction_in_loop[0].callee_name,
            "HttpClient::new"
        );
        // `Vec::with_capacity`/`Vec::new` are explicitly not expensive.
        assert!(cheap_allocs.resource_construction_in_loop.is_empty());
    }

    #[test]
    fn flags_lock_acquisition_in_a_loop_but_not_hoisted_out() {
        let rec = extract_str(
            r#"
            fn per_iteration(mutex: &std::sync::Mutex<i32>, items: &[i32]) {
                for i in items {
                    let mut guard = mutex.lock().unwrap();
                    *guard += i;
                }
            }

            fn hoisted(mutex: &std::sync::Mutex<i32>, items: &[i32]) {
                let mut guard = mutex.lock().unwrap();
                for i in items {
                    *guard += i;
                }
            }
            "#,
        );
        let per_iteration = rec
            .symbols
            .iter()
            .find(|s| s.name == "per_iteration")
            .unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(per_iteration.lock_in_loop.len(), 1);
        assert_eq!(per_iteration.lock_in_loop[0].callee_name, "lock");
        assert!(hoisted.lock_in_loop.is_empty());
    }

    #[test]
    fn flags_index_zero_insert_in_a_loop_but_not_other_indices() {
        let rec = extract_str(
            r#"
            fn reversed(items: &[i32]) -> Vec<i32> {
                let mut out = Vec::new();
                for i in items {
                    out.insert(0, *i);
                }
                out
            }

            fn appended(items: &[i32]) -> Vec<i32> {
                let mut out = Vec::new();
                for i in items {
                    out.insert(out.len(), *i);
                }
                out
            }
            "#,
        );
        let reversed = rec.symbols.iter().find(|s| s.name == "reversed").unwrap();
        let appended = rec.symbols.iter().find(|s| s.name == "appended").unwrap();

        assert_eq!(reversed.list_insert_zero_in_loop.len(), 1);
        assert_eq!(reversed.list_insert_zero_in_loop[0].variable, "out");
        // Insertion at a non-zero index must not match.
        assert!(appended.list_insert_zero_in_loop.is_empty());
    }

    #[test]
    fn flags_json_parse_calls_in_a_loop_but_not_hoisted_out() {
        let rec = extract_str(
            r#"
            fn parses_each_line(lines: &[&str]) {
                for line in lines {
                    let _v: Value = serde_json::from_str(line).unwrap();
                }
            }

            fn hoisted(lines: &[&str]) {
                let _v: Value = serde_json::from_str(lines[0]).unwrap();
                for line in lines {
                    let _ = line.len();
                }
            }
            "#,
        );
        let parses_each_line = rec
            .symbols
            .iter()
            .find(|s| s.name == "parses_each_line")
            .unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(parses_each_line.json_parse_in_loop.len(), 1);
        assert_eq!(
            parses_each_line.json_parse_in_loop[0].callee_name,
            "serde_json::from_str"
        );
        assert!(hoisted.json_parse_in_loop.is_empty());
    }

    #[test]
    fn flags_regex_compilation_in_a_loop_but_not_hoisted_out() {
        let rec = extract_str(
            r#"
            fn per_iteration(lines: &[&str]) -> usize {
                let mut count = 0;
                for line in lines {
                    let re = Regex::new(r"\d+").unwrap();
                    if re.is_match(line) {
                        count += 1;
                    }
                }
                count
            }

            fn hoisted(lines: &[&str]) -> usize {
                let re = Regex::new(r"\d+").unwrap();
                let mut count = 0;
                for line in lines {
                    if re.is_match(line) {
                        count += 1;
                    }
                }
                count
            }
            "#,
        );
        let per_iteration = rec
            .symbols
            .iter()
            .find(|s| s.name == "per_iteration")
            .unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(per_iteration.regex_compile_in_loop.len(), 1);
        assert_eq!(
            per_iteration.regex_compile_in_loop[0].callee_name,
            "Regex::new"
        );
        assert!(hoisted.regex_compile_in_loop.is_empty());
    }

    #[test]
    fn flags_io_in_a_doubly_nested_loop_but_not_a_single_loop() {
        let rec = extract_str(
            r#"
            fn doubly_nested(rows: &[Vec<&str>]) {
                for row in rows {
                    for cell in row {
                        std::fs::read_to_string(cell).unwrap();
                    }
                }
            }

            fn single_loop(cells: &[&str]) {
                for cell in cells {
                    std::fs::read_to_string(cell).unwrap();
                }
            }
            "#,
        );
        let doubly_nested = rec
            .symbols
            .iter()
            .find(|s| s.name == "doubly_nested")
            .unwrap();
        let single_loop = rec
            .symbols
            .iter()
            .find(|s| s.name == "single_loop")
            .unwrap();

        // Doubly-nested triggers both the plain io_in_loop marker and the
        // depth-2+ nested_loop_with_io marker for the same call.
        assert_eq!(doubly_nested.io_in_loop.len(), 1);
        assert_eq!(doubly_nested.nested_loop_with_io.len(), 1);
        assert_eq!(
            doubly_nested.nested_loop_with_io[0].callee_name,
            "read_to_string"
        );

        // A single loop triggers io_in_loop only, not nested_loop_with_io.
        assert_eq!(single_loop.io_in_loop.len(), 1);
        assert!(single_loop.nested_loop_with_io.is_empty());
    }

    #[test]
    fn flags_nested_loops_over_the_same_collection_but_not_unrelated_ones() {
        let rec = extract_str(
            r#"
            fn all_pairs(items: &[i32]) -> usize {
                let mut n = 0;
                for x in items {
                    for y in items.iter() {
                        if x == y {
                            n += 1;
                        }
                    }
                }
                n
            }

            fn cross_product(rows: &[i32], cols: &[i32]) -> usize {
                let mut n = 0;
                for r in rows {
                    for c in cols {
                        n += (r + c) as usize;
                    }
                }
                n
            }

            fn index_walk(items: &[i32]) -> usize {
                let mut n = 0;
                for i in 0..items.len() {
                    for j in 0..items.len() {
                        n += i + j;
                    }
                }
                n
            }
            "#,
        );
        let all_pairs = rec.symbols.iter().find(|s| s.name == "all_pairs").unwrap();
        let cross_product = rec
            .symbols
            .iter()
            .find(|s| s.name == "cross_product")
            .unwrap();
        let index_walk = rec.symbols.iter().find(|s| s.name == "index_walk").unwrap();

        // `items` and `items.iter()` normalize to the same collection.
        assert_eq!(all_pairs.nested_loop_quadratic.len(), 1);
        assert_eq!(all_pairs.nested_loop_quadratic[0].iterable, "items");

        // Two unrelated collections is a legitimate cross product.
        assert!(cross_product.nested_loop_quadratic.is_empty());

        // Ranges are deliberately excluded -- a doubly-nested index walk
        // is usually a deliberate grid traversal, not an accidental
        // all-pairs scan over one collection.
        assert!(index_walk.nested_loop_quadratic.is_empty());
    }

    #[test]
    fn flags_serial_awaits_in_a_loop_but_not_batched_or_hoisted_ones() {
        let rec = extract_str(
            r#"
            async fn serial(urls: &[&str]) {
                for u in urls {
                    let r = fetch(u).await;
                    drop(r);
                }
            }

            async fn batched(urls: &[&str]) {
                for chunk in urls.chunks(10) {
                    let rs = join_all(chunk.iter().map(|u| fetch(u))).await;
                    drop(rs);
                }
            }

            async fn hoisted(urls: &[&str]) {
                let all = join_all(urls.iter().map(|u| fetch(u))).await;
                for r in &all {
                    drop(r);
                }
            }
            "#,
        );
        let serial = rec.symbols.iter().find(|s| s.name == "serial").unwrap();
        let batched = rec.symbols.iter().find(|s| s.name == "batched").unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(serial.serial_await_in_loop.len(), 1);
        assert_eq!(serial.serial_await_in_loop[0].callee_name, "fetch");

        // Awaiting a concurrency combinator inside a loop is chunked
        // concurrency, not the serial pattern -- deliberately not flagged.
        assert!(batched.serial_await_in_loop.is_empty());
        // The await is outside the loop entirely.
        assert!(hoisted.serial_await_in_loop.is_empty());
    }

    #[test]
    fn flags_blocking_calls_in_an_async_fn_but_not_a_sync_one_or_an_awaited_call() {
        let rec = extract_str(
            r#"
            async fn blocks(path: &str) {
                std::thread::sleep(delay);
                let s = std::fs::read_to_string(path).unwrap();
                drop(s);
            }

            fn sync_is_fine(path: &str) {
                std::thread::sleep(delay);
                let s = std::fs::read_to_string(path).unwrap();
                drop(s);
            }

            async fn uses_async_fs(path: &str) {
                let s = tokio::fs::read_to_string(path).await.unwrap();
                drop(s);
            }
            "#,
        );
        let blocks = rec.symbols.iter().find(|s| s.name == "blocks").unwrap();
        let sync_is_fine = rec
            .symbols
            .iter()
            .find(|s| s.name == "sync_is_fine")
            .unwrap();
        let uses_async_fs = rec
            .symbols
            .iter()
            .find(|s| s.name == "uses_async_fs")
            .unwrap();

        assert_eq!(blocks.blocking_sync_in_async.len(), 2);
        let names: Vec<&str> = blocks
            .blocking_sync_in_async
            .iter()
            .map(|b| b.callee_name.as_str())
            .collect();
        assert!(names.contains(&"thread::sleep"));
        assert!(names.contains(&"fs::read_to_string"));

        // The same calls in a plain `fn` are not this marker's concern.
        assert!(sync_is_fine.blocking_sync_in_async.is_empty());

        // `tokio::fs::read_to_string` reduces to the same two-segment
        // path as the blocking `std::fs` one -- being awaited is what
        // tells them apart.
        assert!(uses_async_fs.blocking_sync_in_async.is_empty());
    }

    #[test]
    fn flags_io_while_a_lock_guard_is_held_but_not_before_it() {
        let rec = extract_str(
            r#"
            fn under_lock(m: &Mutex<u32>, p: &str) {
                let guard = m.lock().unwrap();
                let s = std::fs::read_to_string(p).unwrap();
                drop((guard, s));
            }

            fn io_before_lock(m: &Mutex<u32>, p: &str) {
                let s = std::fs::read_to_string(p).unwrap();
                let guard = m.lock().unwrap();
                drop((guard, s));
            }

            fn no_lock_at_all(p: &str) {
                let s = std::fs::read_to_string(p).unwrap();
                drop(s);
            }
            "#,
        );
        let under_lock = rec.symbols.iter().find(|s| s.name == "under_lock").unwrap();
        let io_before_lock = rec
            .symbols
            .iter()
            .find(|s| s.name == "io_before_lock")
            .unwrap();
        let no_lock_at_all = rec
            .symbols
            .iter()
            .find(|s| s.name == "no_lock_at_all")
            .unwrap();

        assert_eq!(under_lock.blocking_io_under_lock.len(), 1);
        assert_eq!(
            under_lock.blocking_io_under_lock[0].callee_name,
            "read_to_string"
        );

        // Same call, but it completes before the guard is acquired.
        assert!(io_before_lock.blocking_io_under_lock.is_empty());
        assert!(no_lock_at_all.blocking_io_under_lock.is_empty());
    }

    #[test]
    fn flags_a_cartesian_join_sql_literal_but_not_a_properly_joined_one() {
        let rec = extract_str(
            r#"
            fn cartesian(db: &Db) {
                db.query("SELECT * FROM orders, customers").unwrap();
            }

            fn joined(db: &Db) {
                db.query("SELECT * FROM orders o, customers c WHERE o.cust_id = c.id")
                    .unwrap();
            }
            "#,
        );
        let cartesian = rec.symbols.iter().find(|s| s.name == "cartesian").unwrap();
        let joined = rec.symbols.iter().find(|s| s.name == "joined").unwrap();

        assert_eq!(cartesian.sql_cartesian_join.len(), 1);
        assert_eq!(cartesian.sql_cartesian_join[0].tables, "orders, customers");
        assert!(joined.sql_cartesian_join.is_empty());
    }

    #[test]
    fn flags_vec_contains_in_a_loop_but_not_hashset_contains() {
        let rec = extract_str(
            "use std::collections::HashSet;\n\
             fn check(needles: Vec<String>) {\n\
             \x20   let allowed = vec![\"a\", \"b\"];\n\
             \x20   let blocked: HashSet<&str> = HashSet::new();\n\
             \x20   let arr = [1, 2, 3];\n\
             \x20   for n in needles {\n\
             \x20       if allowed.contains(&n) {}\n\
             \x20       if blocked.contains(&n) {}\n\
             \x20       if arr.contains(&1) {}\n\
             \x20   }\n\
             }\n",
        );

        let check = rec.symbols.iter().find(|s| s.name == "check").unwrap();
        let names: Vec<_> = check
            .membership_test_in_loop
            .iter()
            .map(|m| m.collection.as_str())
            .collect();
        // `Vec::contains` and `HashSet::contains` are spelled
        // identically -- only the binding separates the O(n) scan from
        // the O(1) lookup, which is the whole point of the binding map.
        assert_eq!(names, vec!["allowed", "arr"]);
    }

    #[test]
    fn a_declared_type_settles_the_kind_when_the_initializer_does_not() {
        let rec = extract_str(
            "fn check(needles: Vec<String>, xs: Vec<String>) {\n\
             \x20   let seen: HashSet<String> = xs.iter().cloned().collect();\n\
             \x20   let listed: Vec<String> = xs.iter().cloned().collect();\n\
             \x20   let opaque = build();\n\
             \x20   for n in needles {\n\
             \x20       if seen.contains(&n) {}\n\
             \x20       if listed.contains(&n) {}\n\
             \x20       if opaque.contains(&n) {}\n\
             \x20   }\n\
             }\n",
        );

        let check = rec.symbols.iter().find(|s| s.name == "check").unwrap();
        let names: Vec<_> = check
            .membership_test_in_loop
            .iter()
            .map(|m| m.collection.as_str())
            .collect();
        // Both initializers are a bare `.collect()`, which says nothing;
        // the declared type is what tells them apart. `opaque` has
        // neither, so it stays unflagged.
        assert_eq!(names, vec!["listed"]);
    }
}
