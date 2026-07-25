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
        let max_nesting_depth = body
            .map(|b| {
                metrics::max_nesting_depth(b, |n| is_decision(n, self.source), is_nested_function)
            })
            .unwrap_or(0);
        let bumpy_road_bumps = body
            .map(|b| {
                metrics::bumpy_road_bumps(b, |n| is_decision(n, self.source), is_nested_function)
            })
            .unwrap_or(0);
        let complex_conditionals = body
            .map(|b| {
                metrics::complex_conditionals(
                    b,
                    condition_of,
                    |n| is_boolean_operator(n, self.source),
                    is_nested_function,
                )
            })
            .unwrap_or_default();
        let param_count = count_params(func_node);
        let primitive_param_count = metrics::primitive_param_count(
            func_node.child_by_field_name("parameters"),
            |n| param_type(n, self.source),
            is_primitive_type,
        );
        let io_in_loop = body
            .map(|b| {
                metrics::calls_in_loops(
                    b,
                    is_loop,
                    |n| call_expression_callee(n, self.source),
                    is_io_call,
                    is_nested_function,
                )
            })
            .unwrap_or_default();
        let string_concat_in_loop = body
            .map(|b| {
                metrics::string_concats_in_loops(
                    b,
                    is_loop,
                    |n| is_string_concat(n, self.source),
                    is_nested_function,
                )
            })
            .unwrap_or_default();
        let resource_construction_in_loop = body
            .map(|b| {
                metrics::resource_constructions_in_loops(
                    b,
                    is_loop,
                    |n| resource_constructor_callee(n, self.source),
                    is_expensive_constructor,
                    is_nested_function,
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
                    is_nested_function,
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
                    is_nested_function,
                )
            })
            .unwrap_or_default();
        let regex_compile_in_loop = body
            .map(|b| {
                metrics::regex_compiles_in_loops(
                    b,
                    is_loop,
                    |n| resource_constructor_callee(n, self.source),
                    is_regex_compile_call,
                    is_nested_function,
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
                    is_nested_function,
                )
            })
            .unwrap_or_default();
        let nested_loop_quadratic = body
            .map(|b| {
                metrics::quadratic_loop_nestings(
                    b,
                    |n| loop_iterable(n, self.source),
                    is_nested_function,
                )
            })
            .unwrap_or_default();
        let serial_await_in_loop = body
            .map(|b| {
                metrics::serial_awaits_in_loops(
                    b,
                    is_loop,
                    |n| awaited_callee(n, self.source),
                    is_nested_function,
                )
            })
            .unwrap_or_default();
        let array_spread_in_reduce = body
            .map(|b| {
                metrics::array_spreads_in_reduce(
                    b,
                    |n| spread_reduce_accumulator(n, self.source),
                    is_nested_function,
                )
            })
            .unwrap_or_default();
        let sql_cartesian_join = body
            .map(|b| {
                metrics::sql_cartesian_joins(
                    b,
                    |n| string_literal_content(n, self.source),
                    is_nested_function,
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
                    is_nested_function,
                )
            })
            .unwrap_or_default();
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
            // `list_insert_zero_in_loop` is Rust/Python-only per its own
            // acceptance criteria (issue #191) -- unlike the other three
            // loop-body markers, it doesn't extend to TypeScript/JavaScript.
            list_insert_zero_in_loop: Vec::new(),
            json_parse_in_loop,
            regex_compile_in_loop,
            nested_loop_with_io,
            nested_loop_quadratic,
            serial_await_in_loop,
            // `pd_concat_in_loop` is Python-only (issue #192): pandas
            // is a Python library with no equivalent in this port's
            // other supported languages.
            pd_concat_in_loop: Vec::new(),
            blocking_sync_in_async: Vec::new(),
            blocking_io_under_lock: Vec::new(),
            array_spread_in_reduce,
            sql_cartesian_join,
            defer_in_loop: Vec::new(),
            goroutine_in_unbounded_loop: Vec::new(),
            membership_test_in_loop,
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

/// The condition sub-expression of an `if`/`while`.
fn condition_of(n: Node) -> Option<Node> {
    match n.kind() {
        "if_statement" | "while_statement" => n.child_by_field_name("condition"),
        _ => None,
    }
}

/// Loop constructs for `io_in_loop` (issue #177): a subset of
/// `is_decision`'s node kinds, excluding the branching-but-not-repeating
/// ones (`if`/`catch`/ternary/`switch` case).
fn is_loop(n: Node) -> bool {
    matches!(
        n.kind(),
        "for_statement" | "for_in_statement" | "while_statement" | "do_statement"
    )
}

/// Array-returning methods recognized as producing a list, for
/// `membership_test_in_loop` (issue #182). Deliberately short: every
/// entry here returns a new array in every standard JS implementation,
/// so none of them can silently mislabel a `Set` as a list.
const ARRAY_PRODUCING_METHODS: &[&str] = &["map", "filter", "split", "concat", "slice", "flat"];

/// A `const name = <collection>` declarator whose initializer shape
/// settles the collection's kind, for `membership_test_in_loop`
/// (issue #182). `None` when the shape doesn't answer the question --
/// see `metrics::list_membership_tests_in_loops` for why unknown
/// bindings are dropped rather than guessed.
fn collection_binding(n: Node, source: &str) -> Option<(String, metrics::CollectionKind)> {
    if n.kind() != "variable_declarator" {
        return None;
    }
    let name_node = n.child_by_field_name("name")?;
    if name_node.kind() != "identifier" {
        return None;
    }
    let value = n.child_by_field_name("value")?;
    let kind = match value.kind() {
        "array" => metrics::CollectionKind::List,
        "new_expression" => {
            let ctor = value.child_by_field_name("constructor")?;
            match text(ctor, source) {
                "Set" | "Map" | "WeakSet" | "WeakMap" => metrics::CollectionKind::NotList,
                "Array" => metrics::CollectionKind::List,
                _ => return None,
            }
        }
        "call_expression" => {
            let func = value.child_by_field_name("function")?;
            if func.kind() != "member_expression" {
                return None;
            }
            let property = func.child_by_field_name("property")?;
            let name = text(property, source);
            // `Object.keys(..)`/`Object.values(..)` and `Array.from(..)`
            // all return arrays, as does every method in the table.
            if ARRAY_PRODUCING_METHODS.contains(&name) || matches!(name, "keys" | "values" | "from")
            {
                metrics::CollectionKind::List
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some((text(name_node, source).to_string(), kind))
}

/// An `xs.includes(x)` / `xs.indexOf(x)` call, for
/// `membership_test_in_loop` (issue #182). `Set.has(..)` is deliberately
/// absent -- it's already the O(1) form this marker recommends. Strings
/// share both method names, but a string binding never resolves to
/// `CollectionKind::List`, so substring checks are filtered out by the
/// binding map rather than needing their own exclusion here.
fn membership_target(n: Node, source: &str) -> Option<metrics::MembershipTarget> {
    if n.kind() != "call_expression" {
        return None;
    }
    let func = n.child_by_field_name("function")?;
    if func.kind() != "member_expression" {
        return None;
    }
    let property = func.child_by_field_name("property")?;
    if !matches!(text(property, source), "includes" | "indexOf") {
        return None;
    }
    let receiver = func.child_by_field_name("object")?;
    match receiver.kind() {
        "array" => Some(metrics::MembershipTarget::InlineList),
        "identifier" => Some(metrics::MembershipTarget::Named(
            text(receiver, source).to_string(),
        )),
        _ => None,
    }
}

/// The callee of an awaited async call, for `serial_await_in_loop`
/// (issue #181): `fetch` for `await fetch(u)`. `None` for a non-await
/// node, for an await whose operand isn't a call (`await somePromise`),
/// and for awaits of the concurrency combinators that *are* the fix --
/// `await Promise.all(..)` inside a loop is chunked concurrency, not
/// the serial pattern this flags.
fn awaited_callee(node: Node, source: &str) -> Option<String> {
    if node.kind() != "await_expression" {
        return None;
    }
    let awaited = node.named_child(0)?;
    if awaited.kind() != "call_expression" {
        return None;
    }
    // Match the combinator on its *qualified* `Promise.all` form: a bare
    // `all`/`race` would be far too generic, and `call_target_name`'s
    // last-property extraction alone can't tell them apart.
    if let Some(qualified) = qualified_call_name(awaited, source) {
        if is_concurrency_combinator(&qualified) {
            return None;
        }
    }
    Some(call_target_name(
        awaited.child_by_field_name("function")?,
        source,
    ))
}

/// `Promise` combinators that batch a whole set of awaits into one
/// concurrent wait -- the fix `serial_await_in_loop` points at, so
/// awaiting one is never itself the problem.
fn is_concurrency_combinator(name: &str) -> bool {
    matches!(
        name,
        "Promise.all" | "Promise.allSettled" | "Promise.race" | "Promise.any"
    )
}

/// A `.reduce(..)`/`.reduceRight(..)` call whose callback returns an
/// array literal spreading its own accumulator, for
/// `array_spread_in_reduce` (issue #194): `(acc, x) => [...acc, x]`.
/// Returns the accumulator's parameter name.
///
/// `[...acc, x]` copies the entire accumulator on every step, so a
/// linear fold becomes quadratic -- a subtle trap, because the spread
/// reads as idiomatic immutable-style JS and gives no visual signal of
/// the copy. The mutate-and-return form (`acc.push(x); return acc`) is
/// the fix and never matches here.
fn spread_reduce_accumulator(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    if func.kind() != "member_expression" {
        return None;
    }
    let property = func.child_by_field_name("property")?;
    if !matches!(text(property, source), "reduce" | "reduceRight") {
        return None;
    }
    let callback = node.child_by_field_name("arguments")?.named_child(0)?;
    if !matches!(callback.kind(), "arrow_function" | "function_expression") {
        return None;
    }
    let accumulator = callback_first_param(callback, source)?;
    let returned = callback_return_expression(callback)?;
    if returned.kind() != "array" {
        return None;
    }
    let mut cursor = returned.walk();
    let spreads_accumulator = returned.named_children(&mut cursor).any(|element| {
        element.kind() == "spread_element"
            && element
                .named_child(0)
                .is_some_and(|inner| text(inner, source) == accumulator)
    });
    if spreads_accumulator {
        Some(accumulator)
    } else {
        None
    }
}

/// A callback's first parameter name -- the accumulator, for a `reduce`
/// callback. Handles both `(acc, x) => ..` (a `formal_parameters` list)
/// and a single-parameter `acc => ..` (a bare identifier).
fn callback_first_param(callback: Node, source: &str) -> Option<String> {
    if let Some(single) = callback.child_by_field_name("parameter") {
        return Some(text(single, source).to_string());
    }
    let params = callback.child_by_field_name("parameters")?;
    Some(text(params.named_child(0)?, source).to_string())
}

/// The expression a callback returns: the body itself for an
/// expression-bodied arrow (`=> [...acc, x]`), or the value of a
/// top-level `return` in a block body.
///
/// Only top-level returns are considered -- a `return` nested inside an
/// `if` isn't found. Coarse in the safe direction: it under-reports
/// rather than guessing about conditional accumulator shapes.
fn callback_return_expression<'a>(callback: Node<'a>) -> Option<Node<'a>> {
    let body = callback.child_by_field_name("body")?;
    if body.kind() != "statement_block" {
        return Some(body);
    }
    let mut cursor = body.walk();
    let returned = body
        .named_children(&mut cursor)
        .find(|c| c.kind() == "return_statement");
    returned.and_then(|r| r.named_child(0))
}

/// The text inside a string-literal node, for `sql_cartesian_join`
/// (issue #195). Covers quoted strings and template literals. Returns `None` for any other node.
fn string_literal_content(node: Node, source: &str) -> Option<String> {
    if !matches!(node.kind(), "string" | "template_string") {
        return None;
    }
    let mut cursor = node.walk();
    let content = node
        .named_children(&mut cursor)
        .find(|c| matches!(c.kind(), "string_content" | "string_fragment"));
    content.map(|c| text(c, source).to_string())
}

/// The base collection a `for...of`/`for...in` loop iterates over, for
/// `nested_loop_quadratic` (issue #187): `items` for all of
/// `for (const x of items)`, `for (const x of items.values())`, and
/// `for (const k of Object.keys(items))`. `None` for `while`/`do` and
/// for a C-style `for (let i = 0; ...)` (both a different grammar node
/// with no single iterable to compare, and the deliberate index-walk
/// shape this marker isn't after), and for any iterable that doesn't
/// normalize down to a plain identifier.
fn loop_iterable(node: Node, source: &str) -> Option<String> {
    if node.kind() != "for_in_statement" {
        return None;
    }
    base_collection_name(node.child_by_field_name("right")?, source)
}

/// Peel a same-collection view method (`.values()`/`.keys()`/
/// `.entries()`/`.slice()`) off an iterable expression, returning the
/// underlying identifier if one is left.
fn base_collection_name(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" => Some(text(node, source).to_string()),
        "call_expression" => {
            let func = node.child_by_field_name("function")?;
            if func.kind() != "member_expression" {
                return None;
            }
            let object = func.child_by_field_name("object")?;
            let property = func.child_by_field_name("property")?;
            let property_name = text(property, source);
            // `Object.keys(x)`/`.values(x)`/`.entries(x)` describe *x*,
            // not the `Object` global -- peeling to the receiver here
            // would collapse every such loop to the name `Object` and
            // make two unrelated collections compare equal.
            if text(object, source) == "Object"
                && matches!(property_name, "keys" | "values" | "entries")
            {
                let arguments = node.child_by_field_name("arguments")?;
                return base_collection_name(arguments.named_child(0)?, source);
            }
            if !matches!(property_name, "values" | "keys" | "entries" | "slice") {
                return None;
            }
            base_collection_name(object, source)
        }
        _ => None,
    }
}

/// If `node` is a `call_expression`, the callee name to match against
/// `is_io_call` -- same extraction `call_target_name` already does for
/// the file's general call graph, applied here to a single node.
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
/// the same last-property name `call_target_name` already uses for the
/// general call graph, so it can't tell `fs.readFile` from an unrelated
/// `readFile` method on some other object, and it can't recognize I/O
/// hidden behind a wrapper function this table doesn't name.
fn is_io_call(name: &str) -> bool {
    matches!(
        name,
        "readFile" | "readFileSync" | "writeFile" | "writeFileSync" | "fetch" | "query" | "execute"
    )
}

/// A string-append expression for `string_concat_in_loop` (issue #178):
/// `s += other` (`augmented_assignment_expression`) or `s = s + other`
/// (`assignment_expression` whose right side is a `+` `binary_expression`
/// naming `s` on one side). JS/TS has no dedicated mutating string-append
/// method (strings are immutable), so those two shapes are the whole
/// pattern here. Returns the appended-onto variable's name.
fn is_string_concat(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "augmented_assignment_expression" => {
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
        _ => None,
    }
}

/// If `node` is a `call_expression` or `new_expression`, the callee/
/// constructor name to match against `is_expensive_constructor` --
/// JS/TS resources are typically constructed via `new Thing(...)`
/// (`new_expression`, not a call), so this checks both shapes rather
/// than reusing `call_expression_callee` (which only handles calls).
fn resource_constructor_callee(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "call_expression" => node
            .child_by_field_name("function")
            .map(|f| call_target_name(f, source)),
        "new_expression" => node
            .child_by_field_name("constructor")
            .map(|f| call_target_name(f, source)),
        _ => None,
    }
}

/// A small fixed table of constructor-shaped names recognized as building
/// an expensive resource (an HTTP client, a connection/thread pool) for
/// `resource_construction_in_loop` (issue #179) -- heuristic and coarse,
/// like `is_io_call`: matching on a bare class/constructor name means it
/// can't tell a project's own `Client` class from an unrelated one of the
/// same name. Deliberately excludes regex construction (`new RegExp`) --
/// reserved for `regex_compile_in_loop` (issue #188).
fn is_expensive_constructor(name: &str) -> bool {
    matches!(name, "ThreadPool" | "Pool" | "HttpClient" | "Client")
}

/// A small fixed table of lock-acquisition method names for
/// `lock_in_loop` (issue #180): JS has no native mutex, but common
/// userland lock libraries (e.g. `async-mutex`) expose an `.acquire()`
/// method, mirroring the Python shape.
fn is_lock_call(name: &str) -> bool {
    matches!(name, "acquire")
}

/// If `node` is a `call_expression` whose target is `object.property(...)`,
/// return the qualified `object.property` name (e.g. `JSON.parse`) rather
/// than just the bare property `call_expression_callee` extracts -- needed
/// for `json_parse_in_loop` (issue #193) since a bare `parse` would be
/// dangerously generic (`Date.parse`, `parseInt`-style helpers, any other
/// `.parse()` method).
fn qualified_call_name(node: Node, source: &str) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    if func.kind() != "member_expression" {
        return None;
    }
    let object = func.child_by_field_name("object")?;
    let property = func.child_by_field_name("property")?;
    Some(format!(
        "{}.{}",
        text(object, source),
        text(property, source)
    ))
}

/// A small fixed table of `Object.method` paths recognized as parsing a
/// JSON payload for `json_parse_in_loop` (issue #193) -- heuristic and
/// coarse, like `is_io_call`.
fn is_json_parse_call(name: &str) -> bool {
    matches!(name, "JSON.parse")
}

/// A small fixed table of constructor names recognized as compiling a
/// regex for `regex_compile_in_loop` (issue #188) -- heuristic and
/// coarse, like `is_io_call`. Unlike `is_json_parse_call`, a bare
/// `RegExp` is already distinctive enough (no qualified form needed),
/// same reasoning as `is_expensive_constructor`'s bare class-name match.
fn is_regex_compile_call(name: &str) -> bool {
    matches!(name, "RegExp")
}

/// A parameter's declared type annotation as source text. TypeScript-only:
/// `required_parameter`/`optional_parameter` (and their `type_annotation`
/// child) are TS grammar node kinds that plain JS trees never produce, so
/// this naturally returns `None` for every parameter in a `.js` file,
/// giving `primitive_param_count` the same "not implemented for this
/// language" zero every other JS-only marker already defaults to.
fn param_type(n: Node, source: &str) -> Option<String> {
    if !matches!(n.kind(), "required_parameter" | "optional_parameter") {
        return None;
    }
    let annotation = n.child_by_field_name("type")?;
    let mut cursor = annotation.walk();
    let type_node = annotation.named_children(&mut cursor).next()?;
    Some(text(type_node, source).to_string())
}

/// The primitives "primitive obsession" flags in TypeScript: `string`,
/// `number`, `boolean` — not `any`/`unknown`/`void`/etc., which aren't the
/// classic smell target.
fn is_primitive_type(t: &str) -> bool {
    matches!(t, "string" | "number" | "boolean")
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
    fn counts_bare_primitive_typed_parameters_but_not_domain_or_untyped() {
        let rec = extract_ts(
            "interface UserId {}\n\nfunction obsessed(name: string, age: number, active: boolean): boolean {\n  return active;\n}\n\nfunction domainTyped(id: UserId, extra): boolean {\n  return true;\n}\n",
        );
        let obsessed = rec.symbols.iter().find(|s| s.name == "obsessed").unwrap();
        assert_eq!(obsessed.primitive_param_count, 3);

        let domain_typed = rec
            .symbols
            .iter()
            .find(|s| s.name == "domainTyped")
            .unwrap();
        // UserId is a domain type and `extra` has no type annotation at
        // all -- neither counts.
        assert_eq!(domain_typed.primitive_param_count, 0);

        // Plain JS has no type annotations at all -- always 0, same as
        // every other language this marker isn't implemented for yet.
        let js_rec = extract_js("function plain(name, age) {\n  return name;\n}\n");
        let plain = js_rec.symbols.iter().find(|s| s.name == "plain").unwrap();
        assert_eq!(plain.primitive_param_count, 0);
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

    #[test]
    fn flags_io_shaped_calls_found_inside_a_loop_body_but_not_outside_one() {
        let rec = extract_js(
            "function hoisted(paths) {\n  const out = [];\n  for (const p of paths) {\n    out.push(fs.readFileSync(p));\n  }\n  return out;\n}\n\nfunction fine(items) {\n  let total = 0;\n  for (const i of items) {\n    total += i;\n  }\n  return total;\n}\n",
        );
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();
        let fine = rec.symbols.iter().find(|s| s.name == "fine").unwrap();

        assert_eq!(hoisted.io_in_loop.len(), 1);
        assert_eq!(hoisted.io_in_loop[0].callee_name, "readFileSync");
        assert!(fine.io_in_loop.is_empty());
    }

    #[test]
    fn flags_string_concat_shapes_found_inside_a_loop_body_but_not_outside_one() {
        let rec = extract_js(
            "function compoundAssign(items) {\n  let s = '';\n  for (const i of items) {\n    s += i;\n  }\n  return s;\n}\n\nfunction reassignment(items) {\n  let s = '';\n  for (const i of items) {\n    s = s + i;\n  }\n  return s;\n}\n\nfunction fine(items) {\n  let s = '';\n  for (const i of items) {\n    s += i;\n  }\n  s = s + ' done';\n  return s;\n}\n",
        );
        let compound_assign = rec
            .symbols
            .iter()
            .find(|s| s.name == "compoundAssign")
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
        assert_eq!(fine.string_concat_in_loop.len(), 1);
    }

    #[test]
    fn flags_expensive_constructors_in_a_loop_but_not_cheap_ones() {
        let rec = extract_js(
            "function hoisted(urls) {\n  for (const u of urls) {\n    const client = new HttpClient();\n    client.get(u);\n  }\n}\n\nfunction cheap(items) {\n  const out = [];\n  for (const i of items) {\n    const arr = new Array();\n    out.push(arr);\n  }\n  return out;\n}\n",
        );
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();
        let cheap = rec.symbols.iter().find(|s| s.name == "cheap").unwrap();

        assert_eq!(hoisted.resource_construction_in_loop.len(), 1);
        assert_eq!(
            hoisted.resource_construction_in_loop[0].callee_name,
            "HttpClient"
        );
        assert!(cheap.resource_construction_in_loop.is_empty());
    }

    #[test]
    fn flags_lock_acquisition_in_a_loop_but_not_hoisted_out() {
        let rec = extract_js(
            "function perIteration(lock, items) {\n  for (const i of items) {\n    lock.acquire();\n    doWork(i);\n  }\n}\n\nfunction hoisted(lock, items) {\n  lock.acquire();\n  for (const i of items) {\n    doWork(i);\n  }\n}\n",
        );
        let per_iteration = rec
            .symbols
            .iter()
            .find(|s| s.name == "perIteration")
            .unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(per_iteration.lock_in_loop.len(), 1);
        assert_eq!(per_iteration.lock_in_loop[0].callee_name, "acquire");
        assert!(hoisted.lock_in_loop.is_empty());
    }

    #[test]
    fn flags_json_parse_calls_in_a_loop_but_not_hoisted_out() {
        let rec = extract_js(
            "function parsesEachLine(lines) {\n  for (const line of lines) {\n    JSON.parse(line);\n  }\n}\n\nfunction hoisted(lines) {\n  JSON.parse(lines[0]);\n  for (const line of lines) {\n    doWork(line);\n  }\n}\n",
        );
        let parses_each_line = rec
            .symbols
            .iter()
            .find(|s| s.name == "parsesEachLine")
            .unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(parses_each_line.json_parse_in_loop.len(), 1);
        assert_eq!(
            parses_each_line.json_parse_in_loop[0].callee_name,
            "JSON.parse"
        );
        assert!(hoisted.json_parse_in_loop.is_empty());
    }

    #[test]
    fn flags_regex_compilation_in_a_loop_but_not_hoisted_out() {
        let rec = extract_js(
            "function perIteration(lines) {\n  let count = 0;\n  for (const line of lines) {\n    const re = new RegExp('\\\\d+');\n    if (re.test(line)) {\n      count++;\n    }\n  }\n  return count;\n}\n\nfunction hoisted(lines) {\n  const re = new RegExp('\\\\d+');\n  let count = 0;\n  for (const line of lines) {\n    if (re.test(line)) {\n      count++;\n    }\n  }\n  return count;\n}\n",
        );
        let per_iteration = rec
            .symbols
            .iter()
            .find(|s| s.name == "perIteration")
            .unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(per_iteration.regex_compile_in_loop.len(), 1);
        assert_eq!(per_iteration.regex_compile_in_loop[0].callee_name, "RegExp");
        assert!(hoisted.regex_compile_in_loop.is_empty());
    }

    #[test]
    fn flags_io_in_a_doubly_nested_loop_but_not_a_single_loop() {
        let rec = extract_js(
            "function doublyNested(rows) {\n  for (const row of rows) {\n    for (const cell of row) {\n      readFileSync(cell);\n    }\n  }\n}\n\nfunction singleLoop(cells) {\n  for (const cell of cells) {\n    readFileSync(cell);\n  }\n}\n",
        );
        let doubly_nested = rec
            .symbols
            .iter()
            .find(|s| s.name == "doublyNested")
            .unwrap();
        let single_loop = rec.symbols.iter().find(|s| s.name == "singleLoop").unwrap();

        assert_eq!(doubly_nested.io_in_loop.len(), 1);
        assert_eq!(doubly_nested.nested_loop_with_io.len(), 1);
        assert_eq!(
            doubly_nested.nested_loop_with_io[0].callee_name,
            "readFileSync"
        );

        assert_eq!(single_loop.io_in_loop.len(), 1);
        assert!(single_loop.nested_loop_with_io.is_empty());
    }

    #[test]
    fn flags_nested_loops_over_the_same_collection_but_not_unrelated_ones() {
        let rec = extract_js(
            "function allPairs(items) {\n  let n = 0;\n  for (const x of items) {\n    for (const y of items.values()) {\n      n++;\n    }\n  }\n  return n;\n}\n\nfunction crossProduct(rows, cols) {\n  let n = 0;\n  for (const r of rows) {\n    for (const c of cols) {\n      n++;\n    }\n  }\n  return n;\n}\n\nfunction objectKeys(a, b) {\n  let n = 0;\n  for (const x of Object.keys(a)) {\n    for (const y of Object.keys(b)) {\n      n++;\n    }\n  }\n  return n;\n}\n",
        );
        let all_pairs = rec.symbols.iter().find(|s| s.name == "allPairs").unwrap();
        let cross_product = rec
            .symbols
            .iter()
            .find(|s| s.name == "crossProduct")
            .unwrap();
        let object_keys = rec.symbols.iter().find(|s| s.name == "objectKeys").unwrap();

        // `items` and `items.values()` normalize to the same collection.
        assert_eq!(all_pairs.nested_loop_quadratic.len(), 1);
        assert_eq!(all_pairs.nested_loop_quadratic[0].iterable, "items");

        assert!(cross_product.nested_loop_quadratic.is_empty());
        // `Object.keys(a)`/`Object.keys(b)` must normalize to `a`/`b`,
        // not both to the `Object` global -- otherwise two unrelated
        // collections would falsely compare equal.
        assert!(object_keys.nested_loop_quadratic.is_empty());
    }

    #[test]
    fn flags_spread_accumulator_reduce_but_not_a_mutating_one() {
        let rec = extract_js(
            "function spreads(xs) {\n  return xs.reduce((acc, x) => [...acc, x], []);\n}\n\nfunction spreadsInBlock(xs) {\n  return xs.reduce((acc, x) => { return [...acc, x]; }, []);\n}\n\nfunction mutates(xs) {\n  return xs.reduce((acc, x) => { acc.push(x); return acc; }, []);\n}\n\nfunction sums(xs) {\n  return xs.reduce((acc, x) => acc + x, 0);\n}\n",
        );
        let spreads = rec.symbols.iter().find(|s| s.name == "spreads").unwrap();
        let spreads_in_block = rec
            .symbols
            .iter()
            .find(|s| s.name == "spreadsInBlock")
            .unwrap();
        let mutates = rec.symbols.iter().find(|s| s.name == "mutates").unwrap();
        let sums = rec.symbols.iter().find(|s| s.name == "sums").unwrap();

        // Both the expression-bodied and block-bodied spread forms.
        assert_eq!(spreads.array_spread_in_reduce.len(), 1);
        assert_eq!(spreads.array_spread_in_reduce[0].accumulator, "acc");
        assert_eq!(spreads_in_block.array_spread_in_reduce.len(), 1);

        // The mutate-and-return form is the fix -- never flagged.
        assert!(mutates.array_spread_in_reduce.is_empty());
        // A plain scalar fold returns no array at all.
        assert!(sums.array_spread_in_reduce.is_empty());
    }

    #[test]
    fn flags_serial_awaits_in_a_loop_but_not_batched_or_hoisted_ones() {
        let rec = extract_js(
            "async function serial(urls) {\n  for (const u of urls) {\n    const r = await fetch(u);\n  }\n}\n\nasync function batched(chunks) {\n  for (const chunk of chunks) {\n    const rs = await Promise.all(chunk.map(fetch));\n  }\n}\n\nasync function hoisted(urls) {\n  const all = await Promise.all(urls.map(fetch));\n  for (const r of all) {\n    use(r);\n  }\n}\n",
        );
        let serial = rec.symbols.iter().find(|s| s.name == "serial").unwrap();
        let batched = rec.symbols.iter().find(|s| s.name == "batched").unwrap();
        let hoisted = rec.symbols.iter().find(|s| s.name == "hoisted").unwrap();

        assert_eq!(serial.serial_await_in_loop.len(), 1);
        assert_eq!(serial.serial_await_in_loop[0].callee_name, "fetch");

        // `await Promise.all(..)` inside a loop is chunked concurrency,
        // deliberately not flagged as a serial await.
        assert!(batched.serial_await_in_loop.is_empty());
        assert!(hoisted.serial_await_in_loop.is_empty());
    }

    #[test]
    fn flags_array_includes_in_a_loop_but_not_set_has() {
        let rec = extract_js(
            "function check(needles) {\n\
             \x20 const allowed = [\"a\", \"b\"];\n\
             \x20 const blocked = new Set([\"x\"]);\n\
             \x20 const parts = raw.split(\",\");\n\
             \x20 for (const n of needles) {\n\
             \x20   if (allowed.includes(n)) {}\n\
             \x20   if (blocked.has(n)) {}\n\
             \x20   if (parts.indexOf(n) !== -1) {}\n\
             \x20 }\n\
             }\n",
        );

        let check = rec.symbols.iter().find(|s| s.name == "check").unwrap();
        let names: Vec<_> = check
            .membership_test_in_loop
            .iter()
            .map(|m| m.collection.as_str())
            .collect();
        // `Set.has` is already the O(1) form this marker recommends, so
        // it is never a membership target at all.
        assert_eq!(names, vec!["allowed", "parts"]);
    }

    #[test]
    fn a_string_receiver_is_not_a_list_membership_test() {
        let rec = extract_js(
            "function check(needles) {\n\
             \x20 const banner = \"hello world\";\n\
             \x20 for (const n of needles) {\n\
             \x20   if (banner.includes(n)) {}\n\
             \x20 }\n\
             }\n",
        );

        // Strings share `includes`/`indexOf` with arrays, but a string
        // binding never resolves to a list, so substring checks are
        // filtered out by the binding map rather than a special case.
        let check = rec.symbols.iter().find(|s| s.name == "check").unwrap();
        assert!(check.membership_test_in_loop.is_empty());
    }
}
