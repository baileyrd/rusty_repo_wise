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
                    let io_in_loop = body
                        .map(|b| {
                            metrics::calls_in_loops(
                                b,
                                is_loop,
                                |n| call_expression_callee(n, self.source),
                                is_io_call,
                                |n| n.kind() == "function_definition",
                            )
                        })
                        .unwrap_or_default();
                    let string_concat_in_loop = body
                        .map(|b| {
                            metrics::string_concats_in_loops(
                                b,
                                is_loop,
                                |n| is_string_concat(n, self.source),
                                |n| n.kind() == "function_definition",
                            )
                        })
                        .unwrap_or_default();
                    let resource_construction_in_loop = body
                        .map(|b| {
                            metrics::resource_constructions_in_loops(
                                b,
                                is_loop,
                                |n| call_expression_callee(n, self.source),
                                is_expensive_constructor,
                                |n| n.kind() == "function_definition",
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
                                |n| n.kind() == "function_definition",
                            )
                        })
                        .unwrap_or_default();
                    let list_insert_zero_in_loop = body
                        .map(|b| {
                            metrics::list_inserts_zero_in_loops(
                                b,
                                is_loop,
                                |n| is_list_insert_zero(n, self.source),
                                |n| n.kind() == "function_definition",
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
                                |n| n.kind() == "function_definition",
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
                                |n| n.kind() == "function_definition",
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
                                |n| n.kind() == "function_definition",
                            )
                        })
                        .unwrap_or_default();
                    let nested_loop_quadratic = body
                        .map(|b| {
                            metrics::quadratic_loop_nestings(
                                b,
                                |n| loop_iterable(n, self.source),
                                |n| n.kind() == "function_definition",
                            )
                        })
                        .unwrap_or_default();
                    let serial_await_in_loop = body
                        .map(|b| {
                            metrics::serial_awaits_in_loops(
                                b,
                                is_loop,
                                |n| awaited_callee(n, self.source),
                                |n| n.kind() == "function_definition",
                            )
                        })
                        .unwrap_or_default();
                    let pd_concat_in_loop = body
                        .map(|b| {
                            metrics::pd_concats_in_loops(
                                b,
                                is_loop,
                                |n| qualified_call_name(n, self.source),
                                is_pd_concat_call,
                                |n| n.kind() == "function_definition",
                            )
                        })
                        .unwrap_or_default();
                    let blocking_sync_in_async = if is_async_fn(node) {
                        body.map(|b| {
                            metrics::blocking_calls_in_async(
                                b,
                                |n| blocking_callee(n, self.source),
                                |n| n.kind() == "function_definition",
                            )
                        })
                        .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let blocking_io_under_lock = body
                        .map(|b| collect_io_under_lock(b, self.source))
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
                        primitive_param_count: 0,
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
                        pd_concat_in_loop,
                        blocking_sync_in_async,
                        blocking_io_under_lock,
                        // `array_spread_in_reduce` is TypeScript/JavaScript-only
                        // (issue #194): it targets the JS array method, which has no
                        // equivalent in this port's other languages.
                        array_spread_in_reduce: Vec::new(),
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

/// Loop constructs for `io_in_loop` (issue #177): a subset of
/// `is_decision`'s node kinds, excluding the branching-but-not-repeating
/// ones (`if`/`elif`/`except`/ternary/`match` case).
fn is_loop(n: Node) -> bool {
    matches!(n.kind(), "for_statement" | "while_statement")
}

/// The body block of a `with <lock>:` statement, for
/// `blocking_io_under_lock` (issue #185), or `None` for any other
/// `with`.
///
/// **This is a name-based heuristic and deliberately so.** Python's
/// `with` is generic, and `lock_in_loop` documents the same limitation
/// from the other side: without type information there is no way to
/// tell a lock context manager from a file handle or a database
/// transaction. Rather than skip Python entirely (the issue asks for
/// it), this matches when the context expression's own name looks like a
/// lock -- `with lock:`, `with self._write_lock:`, `with mutex:`,
/// `with threading.Lock():`. It will miss a lock bound to an
/// unconventional name, and could in principle fire on a non-lock that
/// happens to be named one. Rust's side needs no such guess: a guard
/// binding is structurally identifiable.
fn lock_with_body<'a>(node: Node<'a>, source: &str) -> Option<Node<'a>> {
    if node.kind() != "with_statement" {
        return None;
    }
    let mut cursor = node.walk();
    let clause = node
        .named_children(&mut cursor)
        .find(|c| c.kind() == "with_clause")?;
    let mut clause_cursor = clause.walk();
    let looks_like_lock = clause
        .named_children(&mut clause_cursor)
        .filter_map(|item| item.named_child(0))
        .any(|value| context_name(value, source).is_some_and(|n| is_lock_name(&n)));
    if !looks_like_lock {
        return None;
    }
    node.child_by_field_name("body")
}

/// The trailing name of a `with` context expression: `lock` for `lock`,
/// `_write_lock` for `self._write_lock`, `Lock` for `threading.Lock()`.
fn context_name(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" => Some(text(node, source).to_string()),
        "attribute" => Some(text(node.child_by_field_name("attribute")?, source).to_string()),
        "call" => context_name(node.child_by_field_name("function")?, source),
        _ => None,
    }
}

/// Whether a `with` context's name looks like a lock. Coarse by design
/// -- see `lock_with_body`.
fn is_lock_name(name: &str) -> bool {
    let lowered = name.to_lowercase();
    lowered.contains("lock") || lowered.contains("mutex")
}

/// Walk a function body collecting I/O calls inside every `with <lock>:`
/// block, for `blocking_io_under_lock` (issue #185). A nested lock block
/// inside another is not descended into twice: the outer scan already
/// covers its calls, so recursion stops at the first lock block found on
/// a path.
fn collect_io_under_lock(node: Node, source: &str) -> Vec<repowise_core::BlockingIoUnderLockRef> {
    let mut out = Vec::new();
    if let Some(lock_body) = lock_with_body(node, source) {
        out.extend(metrics::ios_inside_lock_block(
            lock_body,
            |n| call_expression_callee(n, source),
            is_io_call,
            |n| n.kind() == "function_definition",
        ));
        return out;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            continue;
        }
        out.extend(collect_io_under_lock(child, source));
    }
    out
}

/// True when this `function_definition` is declared `async def`, for
/// `blocking_sync_in_async` (issue #184). The `async` keyword is an
/// anonymous token child in tree-sitter-python (not a named node), so
/// this walks all children rather than just the named ones.
fn is_async_fn(node: Node) -> bool {
    if node.kind() != "function_definition" {
        return false;
    }
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| c.kind() == "async");
    found
}

/// The callee of a blocking synchronous call, for
/// `blocking_sync_in_async` (issue #184): `time.sleep` for
/// `time.sleep(1)`, `open` for a bare `open(path)`.
///
/// Accepts both the qualified `module.function` form and a bare
/// identifier callee, because the two need different safety bars: a
/// bare `sleep`/`get` would be far too generic to match, but a bare
/// `open(..)` is Python's builtin and distinctive enough. A call
/// written as a *method* (`fh.open()`) yields the qualified form and so
/// never matches the bare-`open` entry.
fn blocking_callee(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call" {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    let name = match func.kind() {
        "attribute" => qualified_call_name(node, source)?,
        "identifier" => text(func, source).to_string(),
        _ => return None,
    };
    is_blocking_call(&name).then_some(name)
}

/// A small fixed table of blocking stdlib/`requests` calls -- heuristic
/// and coarse, like `is_io_call`. Deliberately limited to calls with a
/// clear async replacement, so every hit has an actionable fix.
fn is_blocking_call(name: &str) -> bool {
    matches!(
        name,
        "time.sleep"
            | "requests.get"
            | "requests.post"
            | "requests.put"
            | "requests.delete"
            | "requests.head"
            | "requests.request"
            | "subprocess.run"
            | "subprocess.call"
            | "subprocess.check_output"
            | "os.system"
            | "open"
    )
}

/// A small fixed table of `module.function` paths recognized as a
/// pandas concatenation for `pd_concat_in_loop` (issue #192), matched on
/// the qualified `object.attribute` form.
///
/// Deliberately covers only `pd.concat`/`pandas.concat` and **not** a
/// bare `.append(..)`, even though the issue's own wording names
/// `DataFrame.append` too. Without type information this port cannot
/// tell `DataFrame.append` from `list.append` -- and appending to a
/// *list* inside a loop is precisely the fix this marker recommends, so
/// flagging bare `.append` would penalize the correct pattern far more
/// often than the wrong one. (`DataFrame.append` was also deprecated in
/// pandas 1.4 and removed in 2.0, so its real-world incidence is
/// shrinking regardless.)
fn is_pd_concat_call(name: &str) -> bool {
    matches!(name, "pd.concat" | "pandas.concat")
}

/// The callee of an awaited async call, for `serial_await_in_loop`
/// (issue #181): `fetch` for `await fetch(u)`. `None` for a non-await
/// node, for an await whose operand isn't a call, and for awaits of the
/// concurrency combinators that *are* the fix.
fn awaited_callee(node: Node, source: &str) -> Option<String> {
    if node.kind() != "await" {
        return None;
    }
    let awaited = node.named_child(0)?;
    if awaited.kind() != "call" {
        return None;
    }
    let name = call_target_name(awaited.child_by_field_name("function")?, source);
    if is_concurrency_combinator(&name) {
        return None;
    }
    Some(name)
}

/// `asyncio` combinators that batch a whole set of awaits into one
/// concurrent wait -- the fix `serial_await_in_loop` points at, so
/// awaiting one is never itself the problem. Matched on the bare last
/// attribute (`gather` for both `asyncio.gather(..)` and a
/// `from asyncio import gather` call). `asyncio.wait` is deliberately
/// left out: `wait` is far too generic a bare method name to exclude
/// safely, and missing it only costs a false positive, not a false
/// negative.
fn is_concurrency_combinator(name: &str) -> bool {
    matches!(name, "gather" | "as_completed")
}

/// The base collection a `for` loop iterates over, for
/// `nested_loop_quadratic` (issue #187): `items` for all of
/// `for x in items`, `for x in enumerate(items)`, `for x in
/// sorted(items)`. `None` for `while` (no iterable to compare) and for
/// any iterable that doesn't normalize down to a plain identifier --
/// including `range(n)`, deliberately, since a doubly-nested range loop
/// is usually a deliberate grid/matrix traversal rather than the
/// accidental all-pairs scan over one collection this marker is after.
fn loop_iterable(node: Node, source: &str) -> Option<String> {
    if node.kind() != "for_statement" {
        return None;
    }
    base_collection_name(node.child_by_field_name("right")?, source)
}

/// Peel a wrapping iteration helper (`enumerate`/`sorted`/`reversed`/
/// `list`/`set`/`tuple`) or a same-collection view method
/// (`.values()`/`.keys()`/`.items()`/`.copy()`) off an iterable
/// expression, returning the underlying identifier if one is left. Only
/// wrappers over the *same* underlying collection are peeled, so
/// `items` and `enumerate(items)` compare equal while a genuinely
/// different sequence (`filter(..)`, a comprehension) doesn't normalize
/// at all.
fn base_collection_name(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" => Some(text(node, source).to_string()),
        "call" => {
            let func = node.child_by_field_name("function")?;
            match func.kind() {
                "identifier" => {
                    if !matches!(
                        text(func, source),
                        "enumerate" | "sorted" | "reversed" | "list" | "set" | "tuple"
                    ) {
                        return None;
                    }
                    let args = node.child_by_field_name("arguments")?;
                    base_collection_name(args.named_child(0)?, source)
                }
                "attribute" => {
                    let attr = func.child_by_field_name("attribute")?;
                    if !matches!(text(attr, source), "values" | "keys" | "items" | "copy") {
                        return None;
                    }
                    base_collection_name(func.child_by_field_name("object")?, source)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// If `node` is a `call`, the callee name to match against `is_io_call`
/// -- same extraction `call_target_name` already does for the file's
/// general call graph, applied here to a single node.
fn call_expression_callee(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call" {
        return None;
    }
    node.child_by_field_name("function")
        .map(|f| call_target_name(f, source))
}

/// A small fixed table of I/O-shaped callee names (file, network, or
/// database operations) -- heuristic and coarse, like
/// `repowise_workspace::contracts`'s route-pattern table: it matches on
/// the same last-attribute name `call_target_name` already uses for the
/// general call graph, so it can't tell a DB cursor's `.execute(..)`
/// from an unrelated `execute` method on some other type, and it can't
/// recognize I/O hidden behind a wrapper function this table doesn't name.
fn is_io_call(name: &str) -> bool {
    matches!(
        name,
        "read"
            | "readline"
            | "readlines"
            | "write"
            | "writelines"
            | "execute"
            | "fetchone"
            | "fetchall"
            | "fetchmany"
            | "urlopen"
            | "urlretrieve"
    )
}

/// A string-append expression for `string_concat_in_loop` (issue #178):
/// `s += other` (`augmented_assignment`) or `s = s + other`
/// (`assignment` whose right side is a `+` `binary_operator` naming `s`
/// on one side). Python has no `.push_str`-equivalent mutating string
/// method (strings are immutable), so those two shapes are the whole
/// pattern here. Returns the appended-onto variable's name.
fn is_string_concat(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "augmented_assignment" => {
            let left = node.child_by_field_name("left")?;
            let operator = node.child_by_field_name("operator")?;
            if left.kind() == "identifier" && text(operator, source) == "+=" {
                Some(text(left, source).to_string())
            } else {
                None
            }
        }
        "assignment" => {
            let left = node.child_by_field_name("left")?;
            if left.kind() != "identifier" {
                return None;
            }
            let left_name = text(left, source);
            let right = node.child_by_field_name("right")?;
            if right.kind() != "binary_operator"
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
        _ => None,
    }
}

/// A small fixed table of constructor-shaped callee names recognized as
/// building an expensive resource (an HTTP session, a connection/thread
/// pool) for `resource_construction_in_loop` (issue #179) -- heuristic
/// and coarse, like `is_io_call`. Uses the same last-attribute-segment
/// name `call_expression_callee` already extracts for the general call
/// graph, so it can't tell a class named `Session` for one purpose from
/// an unrelated class of the same name. Deliberately excludes regex
/// construction (`re.compile`) -- reserved for `regex_compile_in_loop`
/// (issue #188) -- and plain, cheap constructors (`list`, `dict`, etc.).
fn is_expensive_constructor(name: &str) -> bool {
    matches!(
        name,
        "Session" | "ThreadPoolExecutor" | "ProcessPoolExecutor" | "Pool"
    )
}

/// A small fixed table of lock-acquisition method names for
/// `lock_in_loop` (issue #180): `threading.Lock`/`RLock`'s `.acquire()`.
/// Python's `with lock:` shape isn't recognized here -- distinguishing a
/// lock context manager from any other `with` statement would need type
/// information this port doesn't have -- so only the explicit
/// `.acquire()` call form is covered.
fn is_lock_call(name: &str) -> bool {
    matches!(name, "acquire")
}

/// For a call whose target is `object.attribute(...)`, return the
/// qualified `object.attribute` name (e.g. `json.loads`) rather than just
/// the bare attribute `call_expression_callee` extracts -- needed for
/// `json_parse_in_loop` (issue #193) since a bare `loads`/`load` would be
/// dangerously generic (`pickle.load`, `yaml.load`, any other `.load()`
/// method).
fn qualified_call_name(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call" {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    if func.kind() != "attribute" {
        return None;
    }
    let object = func.child_by_field_name("object")?;
    let attribute = func.child_by_field_name("attribute")?;
    Some(format!(
        "{}.{}",
        text(object, source),
        text(attribute, source)
    ))
}

/// A small fixed table of `module.function` paths recognized as parsing
/// a JSON payload for `json_parse_in_loop` (issue #193) -- heuristic and
/// coarse, like `is_io_call`.
fn is_json_parse_call(name: &str) -> bool {
    matches!(name, "json.loads" | "json.load")
}

/// A small fixed table of `module.function` paths recognized as
/// compiling a regex for `regex_compile_in_loop` (issue #188) --
/// heuristic and coarse, like `is_io_call`. Uses the qualified
/// `object.attribute` form rather than the bare last-attribute name
/// `is_expensive_constructor` matches on, since a bare `compile` alone
/// is far too generic (many stdlib/third-party modules expose a
/// `.compile()` method unrelated to regexes).
fn is_regex_compile_call(name: &str) -> bool {
    matches!(name, "re.compile")
}

/// A `.insert(0, ...)` call on a bare identifier for
/// `list_insert_zero_in_loop` (issue #191): a `call` whose callee is an
/// `insert` method on an identifier receiver, whose first argument is
/// the literal `0`. Returns the receiver's variable name. Unlike
/// `is_io_call`/`is_lock_call`'s plain name-table shape, this needs to
/// inspect the call's arguments too, so it's a single combined
/// classifier rather than a name lookup.
fn is_list_insert_zero(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call" {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    if func.kind() != "attribute" {
        return None;
    }
    let attribute = func.child_by_field_name("attribute")?;
    if text(attribute, source) != "insert" {
        return None;
    }
    let object = func.child_by_field_name("object")?;
    if object.kind() != "identifier" {
        return None;
    }
    let arguments = node.child_by_field_name("arguments")?;
    let first_arg = arguments.named_child(0)?;
    if first_arg.kind() == "integer" && text(first_arg, source) == "0" {
        Some(text(object, source).to_string())
    } else {
        None
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

    #[test]
    fn flags_io_shaped_calls_found_inside_a_loop_body_but_not_outside_one() {
        let rec = extract_str(
            "def hoisted(paths):\n    out = []\n    for p in paths:\n        f = open(p)\n        out.append(f.read())\n    return out\n\ndef fine(items):\n    total = 0\n    for i in items:\n        total += i\n    return total\n",
        );
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();
        let fine = rec.symbols.iter().find(|s| s.name == "fine").unwrap();

        assert_eq!(hoisted.io_in_loop.len(), 1);
        assert_eq!(hoisted.io_in_loop[0].callee_name, "read");
        assert!(fine.io_in_loop.is_empty());
    }

    #[test]
    fn flags_string_concat_shapes_found_inside_a_loop_body_but_not_outside_one() {
        let rec = extract_str(
            "def compound_assign(items):\n    s = ''\n    for i in items:\n        s += i\n    return s\n\ndef reassignment(items):\n    s = ''\n    for i in items:\n        s = s + i\n    return s\n\ndef fine(items):\n    s = ''\n    for i in items:\n        s += i\n    s = s + ' done'\n    return s\n",
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
        let fine = rec.symbols.iter().find(|s| s.name == "fine").unwrap();

        assert_eq!(compound_assign.string_concat_in_loop.len(), 1);
        assert_eq!(compound_assign.string_concat_in_loop[0].variable, "s");
        assert_eq!(reassignment.string_concat_in_loop.len(), 1);
        assert_eq!(reassignment.string_concat_in_loop[0].variable, "s");
        // The append after the loop is not flagged, only the one inside it.
        assert_eq!(fine.string_concat_in_loop.len(), 1);
    }

    #[test]
    fn flags_expensive_constructors_in_a_loop_but_not_cheap_ones() {
        let rec = extract_str(
            "def hoisted(urls):\n    for u in urls:\n        s = requests.Session()\n        s.get(u)\n\ndef cheap(items):\n    out = []\n    for i in items:\n        d = dict()\n        out.append(d)\n    return out\n",
        );
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();
        let cheap = rec.symbols.iter().find(|s| s.name == "cheap").unwrap();

        assert_eq!(hoisted.resource_construction_in_loop.len(), 1);
        assert_eq!(
            hoisted.resource_construction_in_loop[0].callee_name,
            "Session"
        );
        assert!(cheap.resource_construction_in_loop.is_empty());
    }

    #[test]
    fn flags_lock_acquisition_in_a_loop_but_not_hoisted_out() {
        let rec = extract_str(
            "def per_iteration(lock, items):\n    for i in items:\n        lock.acquire()\n        do_work(i)\n\ndef hoisted(lock, items):\n    lock.acquire()\n    for i in items:\n        do_work(i)\n",
        );
        let per_iteration = rec
            .symbols
            .iter()
            .find(|s| s.name == "per_iteration")
            .unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(per_iteration.lock_in_loop.len(), 1);
        assert_eq!(per_iteration.lock_in_loop[0].callee_name, "acquire");
        assert!(hoisted.lock_in_loop.is_empty());
    }

    #[test]
    fn flags_index_zero_insert_in_a_loop_but_not_other_indices() {
        let rec = extract_str(
            "def reversed_build(items):\n    out = []\n    for i in items:\n        out.insert(0, i)\n    return out\n\ndef appended(items):\n    out = []\n    for i in items:\n        out.insert(len(out), i)\n    return out\n",
        );
        let reversed_build = rec
            .symbols
            .iter()
            .find(|s| s.name == "reversed_build")
            .unwrap();
        let appended = rec.symbols.iter().find(|s| s.name == "appended").unwrap();

        assert_eq!(reversed_build.list_insert_zero_in_loop.len(), 1);
        assert_eq!(reversed_build.list_insert_zero_in_loop[0].variable, "out");
        assert!(appended.list_insert_zero_in_loop.is_empty());
    }

    #[test]
    fn flags_json_parse_calls_in_a_loop_but_not_hoisted_out() {
        let rec = extract_str(
            "import json\n\ndef parses_each_line(lines):\n    for line in lines:\n        json.loads(line)\n\ndef hoisted(lines):\n    json.loads(lines[0])\n    for line in lines:\n        pass\n",
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
            "json.loads"
        );
        assert!(hoisted.json_parse_in_loop.is_empty());
    }

    #[test]
    fn flags_regex_compilation_in_a_loop_but_not_hoisted_out() {
        let rec = extract_str(
            "import re\n\ndef per_iteration(lines):\n    count = 0\n    for line in lines:\n        pattern = re.compile(r'\\d+')\n        if pattern.match(line):\n            count += 1\n    return count\n\ndef hoisted(lines):\n    pattern = re.compile(r'\\d+')\n    count = 0\n    for line in lines:\n        if pattern.match(line):\n            count += 1\n    return count\n",
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
            "re.compile"
        );
        assert!(hoisted.regex_compile_in_loop.is_empty());
    }

    #[test]
    fn flags_io_in_a_doubly_nested_loop_but_not_a_single_loop() {
        let rec = extract_str(
            "def doubly_nested(rows, fh):\n    for row in rows:\n        for cell in row:\n            fh.write(cell)\n\ndef single_loop(cells, fh):\n    for cell in cells:\n        fh.write(cell)\n",
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

        assert_eq!(doubly_nested.io_in_loop.len(), 1);
        assert_eq!(doubly_nested.nested_loop_with_io.len(), 1);
        assert_eq!(doubly_nested.nested_loop_with_io[0].callee_name, "write");

        assert_eq!(single_loop.io_in_loop.len(), 1);
        assert!(single_loop.nested_loop_with_io.is_empty());
    }

    #[test]
    fn flags_nested_loops_over_the_same_collection_but_not_unrelated_ones() {
        let rec = extract_str(
            "def all_pairs(items):\n    n = 0\n    for x in items:\n        for y in enumerate(items):\n            n += 1\n    return n\n\ndef cross_product(rows, cols):\n    n = 0\n    for r in rows:\n        for c in cols:\n            n += 1\n    return n\n\ndef index_walk(items):\n    n = 0\n    for i in range(len(items)):\n        for j in range(len(items)):\n            n += 1\n    return n\n",
        );
        let all_pairs = rec.symbols.iter().find(|s| s.name == "all_pairs").unwrap();
        let cross_product = rec
            .symbols
            .iter()
            .find(|s| s.name == "cross_product")
            .unwrap();
        let index_walk = rec.symbols.iter().find(|s| s.name == "index_walk").unwrap();

        // `items` and `enumerate(items)` normalize to the same collection.
        assert_eq!(all_pairs.nested_loop_quadratic.len(), 1);
        assert_eq!(all_pairs.nested_loop_quadratic[0].iterable, "items");

        assert!(cross_product.nested_loop_quadratic.is_empty());
        // `range(..)` is deliberately excluded, same as Rust's ranges.
        assert!(index_walk.nested_loop_quadratic.is_empty());
    }

    #[test]
    fn flags_serial_awaits_in_a_loop_but_not_batched_or_hoisted_ones() {
        let rec = extract_str(
            "import asyncio\n\nasync def serial(urls):\n    for u in urls:\n        r = await fetch(u)\n\nasync def batched(chunks):\n    for chunk in chunks:\n        rs = await asyncio.gather(*[fetch(u) for u in chunk])\n\nasync def hoisted(urls):\n    all_rs = await asyncio.gather(*[fetch(u) for u in urls])\n    for r in all_rs:\n        pass\n",
        );
        let serial = rec.symbols.iter().find(|s| s.name == "serial").unwrap();
        let batched = rec.symbols.iter().find(|s| s.name == "batched").unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(serial.serial_await_in_loop.len(), 1);
        assert_eq!(serial.serial_await_in_loop[0].callee_name, "fetch");

        // `asyncio.gather` is a concurrency combinator -- not flagged.
        assert!(batched.serial_await_in_loop.is_empty());
        assert!(hoisted.serial_await_in_loop.is_empty());
    }

    #[test]
    fn flags_pd_concat_in_a_loop_but_not_after_it_or_a_bare_list_append() {
        let rec = extract_str(
            "import pandas as pd\n\ndef grows_in_loop(rows):\n    df = pd.DataFrame()\n    for r in rows:\n        df = pd.concat([df, r])\n    return df\n\ndef concat_once(rows):\n    parts = []\n    for r in rows:\n        parts.append(r)\n    return pd.concat(parts)\n",
        );
        let grows_in_loop = rec
            .symbols
            .iter()
            .find(|s| s.name == "grows_in_loop")
            .unwrap();
        let concat_once = rec
            .symbols
            .iter()
            .find(|s| s.name == "concat_once")
            .unwrap();

        assert_eq!(grows_in_loop.pd_concat_in_loop.len(), 1);
        assert_eq!(grows_in_loop.pd_concat_in_loop[0].callee_name, "pd.concat");

        // The recommended fix: collect into a list in the loop, concat
        // once after it. The `parts.append(r)` inside the loop must NOT
        // be flagged -- that's the fix, not the problem -- and the
        // `pd.concat` is outside the loop entirely.
        assert!(concat_once.pd_concat_in_loop.is_empty());
    }

    #[test]
    fn flags_blocking_calls_in_an_async_def_but_not_a_sync_one() {
        let rec = extract_str(
            "import time\nimport requests\n\nasync def blocks(url):\n    time.sleep(1)\n    r = requests.get(url)\n    return r\n\ndef sync_is_fine(url):\n    time.sleep(1)\n    return requests.get(url)\n\nasync def opens_a_file(path):\n    fh = open(path)\n    return fh\n",
        );
        let blocks = rec.symbols.iter().find(|s| s.name == "blocks").unwrap();
        let sync_is_fine = rec
            .symbols
            .iter()
            .find(|s| s.name == "sync_is_fine")
            .unwrap();
        let opens_a_file = rec
            .symbols
            .iter()
            .find(|s| s.name == "opens_a_file")
            .unwrap();

        assert_eq!(blocks.blocking_sync_in_async.len(), 2);
        let names: Vec<&str> = blocks
            .blocking_sync_in_async
            .iter()
            .map(|b| b.callee_name.as_str())
            .collect();
        assert!(names.contains(&"time.sleep"));
        assert!(names.contains(&"requests.get"));

        // The same calls in a plain `def` are not this marker's concern.
        assert!(sync_is_fine.blocking_sync_in_async.is_empty());

        // A bare builtin `open(..)` is distinctive enough to match on
        // its own, unlike a bare `sleep`/`get`.
        assert_eq!(opens_a_file.blocking_sync_in_async.len(), 1);
        assert_eq!(opens_a_file.blocking_sync_in_async[0].callee_name, "open");
    }

    #[test]
    fn flags_io_inside_a_with_lock_block_but_not_outside_one() {
        let rec = extract_str(
            "def under_lock(lock, fh):\n    with lock:\n        fh.write('x')\n\ndef io_outside(lock, fh):\n    fh.write('x')\n    with lock:\n        pass\n\ndef other_with(cm, fh):\n    with cm:\n        fh.write('x')\n",
        );
        let under_lock = rec.symbols.iter().find(|s| s.name == "under_lock").unwrap();
        let io_outside = rec.symbols.iter().find(|s| s.name == "io_outside").unwrap();
        let other_with = rec.symbols.iter().find(|s| s.name == "other_with").unwrap();

        assert_eq!(under_lock.blocking_io_under_lock.len(), 1);
        assert_eq!(under_lock.blocking_io_under_lock[0].callee_name, "write");

        assert!(io_outside.blocking_io_under_lock.is_empty());
        // `with cm:` doesn't look like a lock -- the name-based
        // heuristic deliberately stays quiet rather than guess.
        assert!(other_with.blocking_io_under_lock.is_empty());
    }
}
