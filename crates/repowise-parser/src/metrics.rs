//! Language-agnostic per-symbol metrics computed directly from the AST:
//! cyclomatic complexity, max nesting depth, a "bumpy road" nested-block
//! count, per-condition boolean-operator counting, parameter count, and
//! a duplicate-code body hash. These feed `repowise-health`'s
//! deterministic scoring.

use repowise_core::{
    ArraySpreadInReduceRef, BlockingIoUnderLockRef, BlockingSyncInAsyncRef, ComplexConditionalRef,
    DeferInLoopRef, GoroutineInUnboundedLoopRef, IoInLoopRef, JsonParseInLoopRef,
    ListInsertZeroInLoopRef, LockInLoopRef, NestedLoopQuadraticRef, NestedLoopWithIoRef,
    PdConcatInLoopRef, RegexCompileInLoopRef, ResourceConstructionInLoopRef, SerialAwaitInLoopRef,
    SqlCartesianJoinRef, StringConcatInLoopRef,
};
use std::hash::{Hash, Hasher};
use tree_sitter::Node;

/// Bodies shorter than this (in lines) aren't hashed for duplicate
/// detection — trivial one-liners (getters, `{ 0 }`) match too often to
/// be a useful signal.
const MIN_DUPLICATE_LINES: usize = 4;

/// McCabe-style cyclomatic complexity: starts at 1 (one path through the
/// function), +1 per decision point as classified by `is_decision`.
/// Recursion stops at nodes matched by `is_nested_function` so a nested
/// function/closure's branches aren't double-counted into the enclosing
/// symbol's complexity (the nested one gets its own symbol + complexity).
pub fn cyclomatic_complexity(
    body: Node,
    is_decision: impl Fn(Node) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> usize {
    let mut count = 1usize;
    let mut stack = vec![body];
    while let Some(n) = stack.pop() {
        if is_decision(n) {
            count += 1;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if is_nested_function(child) {
                continue;
            }
            stack.push(child);
        }
    }
    count
}

/// Maximum nesting depth of decision-classified blocks within `body`
/// (0 = no nested blocks at all). Unlike `cyclomatic_complexity` (which
/// counts *how many* decision points there are, flat), this tracks *how
/// deep* they're nested: a recursive walk that increments depth only
/// when descending into a child classified by `is_decision`, and
/// returns the maximum depth reached anywhere in the subtree. Recursion
/// stops at `is_nested_function`-matched nodes, same as
/// `cyclomatic_complexity`, so a nested function/closure's own nesting
/// doesn't inflate the enclosing symbol's depth.
pub fn max_nesting_depth(
    body: Node,
    is_decision: impl Fn(Node) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> usize {
    fn walk(
        node: Node,
        depth: usize,
        is_decision: &dyn Fn(Node) -> bool,
        is_nested_function: &dyn Fn(Node) -> bool,
    ) -> usize {
        let mut max_depth = depth;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_nested_function(child) {
                continue;
            }
            let child_depth = if is_decision(child) { depth + 1 } else { depth };
            let reached = walk(child, child_depth, is_decision, is_nested_function);
            if reached > max_depth {
                max_depth = reached;
            }
        }
        max_depth
    }
    walk(body, 0, &is_decision, &is_nested_function)
}

/// Minimum nesting depth (see `max_nesting_depth`) a decision node must
/// reach to count as a "bump" in `bumpy_road_bumps` — a depth-1 (i.e.
/// un-nested) `if`/`for`/etc. is just ordinary branching, already
/// captured by `cyclomatic_complexity`; a bump specifically means a
/// block nested *inside* another one.
const BUMP_MIN_DEPTH: usize = 2;

/// "Bumpy Road" count: the number of distinct nested-block regions at
/// or beyond `BUMP_MIN_DEPTH` within `body`. Complements
/// `max_nesting_depth` (which only reports the single deepest point):
/// a function with three separate two-level-deep `if`s reads worse than
/// one with a single two-level-deep `if`, even though both have the same
/// max nesting depth — `max_nesting_depth` alone can't tell them apart,
/// but this can.
///
/// Counting rule: only *leaf* decision nodes count — a decision node
/// with no further decision node nested inside it (before hitting an
/// `is_nested_function` boundary). A linear chain (`if` containing
/// `if` containing `if`) has exactly one leaf (the innermost `if`) and
/// so counts as a single bump, not three — it's one deep block, not
/// several scattered ones. Three separate sibling `if`s, each with one
/// level of nesting inside, have three leaves and count as three bumps.
/// This is computed in one post-order pass: `walk` returns whether the
/// subtree it just visited contained any decision node at all, which is
/// exactly "does this decision node have further nesting inside it".
pub fn bumpy_road_bumps(
    body: Node,
    is_decision: impl Fn(Node) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> usize {
    fn walk(
        node: Node,
        depth: usize,
        is_decision: &dyn Fn(Node) -> bool,
        is_nested_function: &dyn Fn(Node) -> bool,
        bumps: &mut usize,
    ) -> bool {
        let mut subtree_has_decision = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_nested_function(child) {
                continue;
            }
            if is_decision(child) {
                subtree_has_decision = true;
                let child_depth = depth + 1;
                let child_has_nested =
                    walk(child, child_depth, is_decision, is_nested_function, bumps);
                if !child_has_nested && child_depth >= BUMP_MIN_DEPTH {
                    *bumps += 1;
                }
            } else {
                let child_has_decision = walk(child, depth, is_decision, is_nested_function, bumps);
                subtree_has_decision |= child_has_decision;
            }
        }
        subtree_has_decision
    }
    let mut bumps = 0;
    walk(body, 0, &is_decision, &is_nested_function, &mut bumps);
    bumps
}

/// A condition chaining at least this many boolean operators (`&&`/`||`
/// and language equivalents) is flagged as a "complex conditional" —
/// already-computed cyclomatic complexity counts each operator as +1
/// toward the *function's* total, but doesn't flag the specific
/// condition as locally hard to read.
const COMPLEX_CONDITIONAL_MIN_OPERATORS: usize = 3;

/// Every `if`/`while`/etc. condition within `body` chaining at least
/// `COMPLEX_CONDITIONAL_MIN_OPERATORS` boolean operators, with the
/// condition's own line and operator count — unlike `cyclomatic_complexity`
/// (a single number for the whole function) or `max_nesting_depth`/
/// `bumpy_road_bumps` (which describe nesting shape), this points at the
/// *specific* expression that's locally hard to read.
///
/// `condition_of` extracts the condition sub-expression from a decision
/// node (e.g. `if_expression` -> its `condition` field); nodes with no
/// condition (a `for` loop's range, a `match` arm) return `None` and are
/// skipped. `is_boolean_operator` classifies a node as a chaining
/// operator (e.g. `binary_expression` with `&&`/`||`) — this is
/// deliberately a *separate* closure from `is_decision`, even though
/// both often check the same node kind, because here we're counting
/// operators *within one condition's own subtree*, not decision points
/// across the whole function body.
pub fn complex_conditionals(
    body: Node,
    condition_of: impl Fn(Node) -> Option<Node>,
    is_boolean_operator: impl Fn(Node) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<ComplexConditionalRef> {
    fn count_operators(node: Node, is_boolean_operator: &dyn Fn(Node) -> bool) -> usize {
        let mut count = usize::from(is_boolean_operator(node));
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            count += count_operators(child, is_boolean_operator);
        }
        count
    }

    fn walk(
        node: Node,
        condition_of: &dyn Fn(Node) -> Option<Node>,
        is_boolean_operator: &dyn Fn(Node) -> bool,
        is_nested_function: &dyn Fn(Node) -> bool,
        out: &mut Vec<ComplexConditionalRef>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_nested_function(child) {
                continue;
            }
            if let Some(condition) = condition_of(child) {
                let operator_count = count_operators(condition, is_boolean_operator);
                if operator_count >= COMPLEX_CONDITIONAL_MIN_OPERATORS {
                    out.push(ComplexConditionalRef {
                        line: condition.start_position().row + 1,
                        operator_count,
                    });
                }
            }
            walk(
                child,
                condition_of,
                is_boolean_operator,
                is_nested_function,
                out,
            );
        }
    }

    let mut out = Vec::new();
    walk(
        body,
        &condition_of,
        &is_boolean_operator,
        &is_nested_function,
        &mut out,
    );
    out
}

/// Every node within `body` for which `classify` returns `Some(value)`
/// (`None` for a non-match), found anywhere inside a loop (`is_loop`),
/// paired with its own line. Tracks a single "currently inside a loop"
/// flag down the whole walk -- unlike `complex_conditionals` (which
/// inspects each decision node's own condition subtree in isolation) --
/// so a match nested inside two loops is still only reported once, at
/// its own line, rather than once per enclosing loop. Shared by every
/// "X found inside a loop" health marker in repowise's Performance-signal
/// cluster (issue #72 and friends, starting with #177's `io_in_loop`):
/// each caller supplies its own per-language `is_loop`/`classify` and
/// maps the `(line, value)` pairs into its own dedicated `Vec<...Ref>`
/// type, so `Symbol`/`Finding` still get a marker-specific, self-describing
/// shape rather than a shared generic one.
fn matches_in_loops<T>(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    classify: impl Fn(Node) -> Option<T>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<(usize, T)> {
    fn walk<T>(
        node: Node,
        in_loop: bool,
        is_loop: &dyn Fn(Node) -> bool,
        classify: &dyn Fn(Node) -> Option<T>,
        is_nested_function: &dyn Fn(Node) -> bool,
        out: &mut Vec<(usize, T)>,
    ) {
        if in_loop {
            if let Some(value) = classify(node) {
                out.push((node.start_position().row + 1, value));
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_nested_function(child) {
                continue;
            }
            let child_in_loop = in_loop || is_loop(child);
            walk(
                child,
                child_in_loop,
                is_loop,
                classify,
                is_nested_function,
                out,
            );
        }
    }
    let mut out = Vec::new();
    walk(
        body,
        false,
        &is_loop,
        &classify,
        &is_nested_function,
        &mut out,
    );
    out
}

/// Every call within `body` recognized as I/O-shaped (`is_io_call`, applied
/// to the name `call_callee` extracts for a call-expression node -- `None`
/// for non-call nodes) that occurs anywhere inside a loop (`is_loop`).
/// See `matches_in_loops` for the shared "currently inside a loop"
/// tracking shape this (and every sibling `*_in_loops` marker function)
/// builds on.
pub fn calls_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    call_callee: impl Fn(Node) -> Option<String>,
    is_io_call: impl Fn(&str) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<IoInLoopRef> {
    matches_in_loops(
        body,
        is_loop,
        |n| call_callee(n).filter(|name| is_io_call(name)),
        is_nested_function,
    )
    .into_iter()
    .map(|(line, callee_name)| IoInLoopRef { line, callee_name })
    .collect()
}

/// Every string-append expression within `body` (`is_string_concat`,
/// applied to each node -- returns the appended-onto variable's name, or
/// `None` if the node isn't a recognized append shape) that occurs
/// anywhere inside a loop (`is_loop`). See `matches_in_loops`.
pub fn string_concats_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    is_string_concat: impl Fn(Node) -> Option<String>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<StringConcatInLoopRef> {
    matches_in_loops(body, is_loop, is_string_concat, is_nested_function)
        .into_iter()
        .map(|(line, variable)| StringConcatInLoopRef { line, variable })
        .collect()
}

/// Every call within `body` recognized as constructing an expensive
/// resource (`is_expensive_constructor`, applied to the name
/// `constructor_callee` extracts -- `None` for non-matching nodes) that
/// occurs anywhere inside a loop (`is_loop`). See `matches_in_loops`.
pub fn resource_constructions_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    constructor_callee: impl Fn(Node) -> Option<String>,
    is_expensive_constructor: impl Fn(&str) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<ResourceConstructionInLoopRef> {
    matches_in_loops(
        body,
        is_loop,
        |n| constructor_callee(n).filter(|name| is_expensive_constructor(name)),
        is_nested_function,
    )
    .into_iter()
    .map(|(line, callee_name)| ResourceConstructionInLoopRef { line, callee_name })
    .collect()
}

/// Every call within `body` recognized as acquiring a mutex/lock
/// (`is_lock_call`, applied to the name `call_callee` extracts -- `None`
/// for non-matching nodes) that occurs anywhere inside a loop (`is_loop`).
/// See `matches_in_loops`.
pub fn locks_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    call_callee: impl Fn(Node) -> Option<String>,
    is_lock_call: impl Fn(&str) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<LockInLoopRef> {
    matches_in_loops(
        body,
        is_loop,
        |n| call_callee(n).filter(|name| is_lock_call(name)),
        is_nested_function,
    )
    .into_iter()
    .map(|(line, callee_name)| LockInLoopRef { line, callee_name })
    .collect()
}

/// Every call within `body` recognized as inserting at index 0 of a
/// list/vector (`is_list_insert_zero`, applied to each node -- returns
/// the list/vector variable's name, or `None` if the node isn't a
/// recognized index-0-insert shape) that occurs anywhere inside a loop
/// (`is_loop`). Unlike `calls_in_loops`/`locks_in_loops` (which filter a
/// plain callee name against a fixed table), this classifier needs to
/// inspect the call's *arguments* too (the first argument must be the
/// literal `0`), so it's a single combined closure rather than a
/// name-table `filter` step. See `matches_in_loops`.
pub fn list_inserts_zero_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    is_list_insert_zero: impl Fn(Node) -> Option<String>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<ListInsertZeroInLoopRef> {
    matches_in_loops(body, is_loop, is_list_insert_zero, is_nested_function)
        .into_iter()
        .map(|(line, variable)| ListInsertZeroInLoopRef { line, variable })
        .collect()
}

/// Every call within `body` recognized as parsing a JSON payload
/// (`is_json_parse_call`, applied to the name `call_callee` extracts --
/// `None` for non-matching nodes) that occurs anywhere inside a loop
/// (`is_loop`). See `matches_in_loops`.
pub fn json_parses_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    call_callee: impl Fn(Node) -> Option<String>,
    is_json_parse_call: impl Fn(&str) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<JsonParseInLoopRef> {
    matches_in_loops(
        body,
        is_loop,
        |n| call_callee(n).filter(|name| is_json_parse_call(name)),
        is_nested_function,
    )
    .into_iter()
    .map(|(line, callee_name)| JsonParseInLoopRef { line, callee_name })
    .collect()
}

pub fn regex_compiles_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    call_callee: impl Fn(Node) -> Option<String>,
    is_regex_compile_call: impl Fn(&str) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<RegexCompileInLoopRef> {
    matches_in_loops(
        body,
        is_loop,
        |n| call_callee(n).filter(|name| is_regex_compile_call(name)),
        is_nested_function,
    )
    .into_iter()
    .map(|(line, callee_name)| RegexCompileInLoopRef { line, callee_name })
    .collect()
}

/// Like `matches_in_loops`, but tracks a running loop-nesting *depth*
/// instead of a single in-loop boolean, and only reports a match once
/// depth reaches `min_depth` or deeper -- for `nested_loop_with_io`
/// (issue #183), which needs to distinguish a call inside one loop from
/// a call inside a loop nested inside another loop, unlike every other
/// loop-body marker built on `matches_in_loops` above, which only cares
/// whether a loop encloses the call at all.
fn matches_in_nested_loops<T>(
    body: Node,
    min_depth: usize,
    is_loop: impl Fn(Node) -> bool,
    classify: impl Fn(Node) -> Option<T>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<(usize, T)> {
    #[allow(clippy::too_many_arguments)]
    fn walk<T>(
        node: Node,
        loop_depth: usize,
        min_depth: usize,
        is_loop: &dyn Fn(Node) -> bool,
        classify: &dyn Fn(Node) -> Option<T>,
        is_nested_function: &dyn Fn(Node) -> bool,
        out: &mut Vec<(usize, T)>,
    ) {
        if loop_depth >= min_depth {
            if let Some(value) = classify(node) {
                out.push((node.start_position().row + 1, value));
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_nested_function(child) {
                continue;
            }
            let child_depth = loop_depth + usize::from(is_loop(child));
            walk(
                child,
                child_depth,
                min_depth,
                is_loop,
                classify,
                is_nested_function,
                out,
            );
        }
    }
    let mut out = Vec::new();
    walk(
        body,
        0,
        min_depth,
        &is_loop,
        &classify,
        &is_nested_function,
        &mut out,
    );
    out
}

/// I/O-shaped calls (the same pattern table `calls_in_loops` uses) found
/// at loop-nesting depth 2 or deeper -- worse than a single-loop
/// `io_in_loop` hit, since it's potentially O(n^2) (or deeper) I/O calls.
pub fn ios_in_nested_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    call_callee: impl Fn(Node) -> Option<String>,
    is_io_call: impl Fn(&str) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<NestedLoopWithIoRef> {
    matches_in_nested_loops(
        body,
        2,
        is_loop,
        |n| call_callee(n).filter(|name| is_io_call(name)),
        is_nested_function,
    )
    .into_iter()
    .map(|(line, callee_name)| NestedLoopWithIoRef { line, callee_name })
    .collect()
}

/// Calls anywhere in a body matching `classify`, skipping nested
/// function bodies. The non-loop counterpart to `matches_in_loops`, for
/// markers whose context is the enclosing *function* rather than an
/// enclosing loop.
fn matches_in_body<T>(
    body: Node,
    classify: impl Fn(Node) -> Option<T>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<(usize, T)> {
    fn walk<T>(
        node: Node,
        classify: &dyn Fn(Node) -> Option<T>,
        is_nested_function: &dyn Fn(Node) -> bool,
        out: &mut Vec<(usize, T)>,
    ) {
        if let Some(value) = classify(node) {
            out.push((node.start_position().row + 1, value));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_nested_function(child) {
                continue;
            }
            walk(child, classify, is_nested_function, out);
        }
    }
    let mut out = Vec::new();
    walk(body, &classify, &is_nested_function, &mut out);
    out
}

/// Matches found *after* a scope-opening marker within the same block,
/// staying in effect for the rest of that block and everything nested
/// inside it. Models a lexically-scoped binding that lives to the end
/// of its block -- Rust's `let guard = m.lock()..;`, where every
/// statement after the binding runs inside the critical section.
///
/// The flag propagates down into children but only turns on for
/// *subsequent* siblings, so each node is visited exactly once and a
/// nested block with its own marker can't double-report calls the outer
/// scope already covered.
fn matches_after_scope_marker<T>(
    body: Node,
    is_scope_marker: impl Fn(Node) -> bool,
    classify: impl Fn(Node) -> Option<T>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<(usize, T)> {
    fn walk<T>(
        node: Node,
        in_scope: bool,
        is_scope_marker: &dyn Fn(Node) -> bool,
        classify: &dyn Fn(Node) -> Option<T>,
        is_nested_function: &dyn Fn(Node) -> bool,
        out: &mut Vec<(usize, T)>,
    ) {
        if in_scope {
            if let Some(value) = classify(node) {
                out.push((node.start_position().row + 1, value));
            }
        }
        let mut in_scope = in_scope;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_nested_function(child) {
                continue;
            }
            walk(
                child,
                in_scope,
                is_scope_marker,
                classify,
                is_nested_function,
                out,
            );
            if is_scope_marker(child) {
                in_scope = true;
            }
        }
    }
    let mut out = Vec::new();
    walk(
        body,
        false,
        &is_scope_marker,
        &classify,
        &is_nested_function,
        &mut out,
    );
    out
}

/// I/O calls made after a lexically-scoped lock binding, for
/// `blocking_io_under_lock` (issue #185) -- Rust's guard shape, where a
/// `let g = m.lock()..;` holds the lock to the end of its block.
pub fn ios_under_lock_binding(
    body: Node,
    is_lock_binding: impl Fn(Node) -> bool,
    io_callee: impl Fn(Node) -> Option<String>,
    is_io_call: impl Fn(&str) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<BlockingIoUnderLockRef> {
    matches_after_scope_marker(
        body,
        is_lock_binding,
        |n| io_callee(n).filter(|name| is_io_call(name)),
        is_nested_function,
    )
    .into_iter()
    .map(|(line, callee_name)| BlockingIoUnderLockRef { line, callee_name })
    .collect()
}

/// I/O calls inside an explicitly-delimited lock block, for
/// `blocking_io_under_lock` (issue #185) -- Python's `with lock:` shape.
/// Called once per lock block; the caller identifies the blocks.
pub fn ios_inside_lock_block(
    lock_block: Node,
    io_callee: impl Fn(Node) -> Option<String>,
    is_io_call: impl Fn(&str) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<BlockingIoUnderLockRef> {
    matches_in_body(
        lock_block,
        |n| io_callee(n).filter(|name| is_io_call(name)),
        is_nested_function,
    )
    .into_iter()
    .map(|(line, callee_name)| BlockingIoUnderLockRef { line, callee_name })
    .collect()
}

/// Clause keywords that terminate a `FROM` or `WHERE` region in the
/// coarse SQL scan below.
const SQL_CLAUSE_KEYWORDS: &[&str] = &[
    " where ", " group ", " order ", " having ", " limit ", " union ", " offset ",
];

/// The region of `lowered` starting just after `keyword` and ending at
/// the next clause keyword (or end of string).
fn sql_clause_after(lowered: &str, keyword: &str) -> Option<(usize, String)> {
    let start = lowered.find(keyword)? + keyword.len();
    let rest = &lowered[start..];
    let end = SQL_CLAUSE_KEYWORDS
        .iter()
        .filter_map(|k| rest.find(k))
        .min()
        .unwrap_or(rest.len());
    Some((start, rest[..end].to_string()))
}

/// Whether a SQL query text looks like an accidental cartesian product:
/// a comma-separated multi-table `FROM` clause without enough join
/// predicates to connect the tables. Returns the table names when so.
///
/// Deliberately a coarse text scan, not a SQL parse -- the same
/// heuristic framing as `repowise_workspace::contracts`' route-pattern
/// table. It only looks at the comma-join form: a `FROM` clause
/// containing an explicit `JOIN` is left alone entirely, since the
/// `ON` predicate that belongs to it is the thing that would need
/// matching up and an explicit join is rarely the accidental case.
/// A query assembled by string concatenation is invisible to it, since
/// only one literal is ever in hand at a time.
pub fn sql_cartesian_join_tables(sql: &str) -> Option<Vec<String>> {
    let lowered = format!(" {} ", sql.to_lowercase().replace(['\n', '\t'], " "));
    if !lowered.contains(" select ") {
        return None;
    }
    let (_, from_clause) = sql_clause_after(&lowered, " from ")?;
    // An explicit `JOIN ... ON` is a different (and usually deliberate)
    // shape than a comma join -- out of scope here.
    if from_clause.contains(" join ") {
        return None;
    }
    let tables: Vec<String> = from_clause
        .split(',')
        .filter_map(|entry| entry.split_whitespace().next().map(str::to_string))
        .filter(|t| !t.is_empty())
        .collect();
    if tables.len() < 2 {
        return None;
    }
    // Each additional table needs at least one predicate tying it to
    // another, so `n` tables need `n - 1` qualified equality predicates.
    let predicates = match sql_clause_after(&lowered, " where ") {
        Some((_, where_clause)) => qualified_equality_predicate_count(&where_clause),
        None => 0,
    };
    if predicates >= tables.len() - 1 {
        return None;
    }
    Some(tables)
}

/// Count `a.b = c.d`-shaped predicates -- both sides qualified, which is
/// what distinguishes a join condition from a plain column filter like
/// `a.id = 5`.
fn qualified_equality_predicate_count(where_clause: &str) -> usize {
    static PREDICATE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = PREDICATE.get_or_init(|| {
        regex::Regex::new(r"\w+\.\w+\s*=\s*\w+\.\w+").expect("static regex is valid")
    });
    re.find_iter(where_clause).count()
}

/// SQL string literals that look like accidental cartesian joins, for
/// `sql_cartesian_join` (issue #195). `string_content` yields the text
/// inside a string-literal node, or `None` for any other node.
pub fn sql_cartesian_joins(
    body: Node,
    string_content: impl Fn(Node) -> Option<String>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<SqlCartesianJoinRef> {
    matches_in_body(
        body,
        |n| {
            string_content(n)
                .and_then(|sql| sql_cartesian_join_tables(&sql))
                .map(|tables| tables.join(", "))
        },
        is_nested_function,
    )
    .into_iter()
    .map(|(line, tables)| SqlCartesianJoinRef { line, tables })
    .collect()
}

/// `.reduce(..)` callbacks spreading their accumulator into a new array,
/// for `array_spread_in_reduce` (issue #194). Scans the whole body --
/// the shape is self-contained in the `reduce` call, so no enclosing
/// loop or function context is involved.
pub fn array_spreads_in_reduce(
    body: Node,
    spread_reduce_accumulator: impl Fn(Node) -> Option<String>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<ArraySpreadInReduceRef> {
    matches_in_body(body, spread_reduce_accumulator, is_nested_function)
        .into_iter()
        .map(|(line, accumulator)| ArraySpreadInReduceRef { line, accumulator })
        .collect()
}

/// Go `defer` statements found inside a loop body, for `defer_in_loop`
/// (issue #189). `defer_callee` returns the deferred call's name for a
/// defer-statement node and `None` for everything else, so the language
/// arm owns both "is this a defer" and "what does it defer" -- there's
/// no name table to filter against here, since the `defer` keyword is
/// the entire signal. See `matches_in_loops`.
pub fn defers_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    defer_callee: impl Fn(Node) -> Option<String>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<DeferInLoopRef> {
    matches_in_loops(body, is_loop, defer_callee, is_nested_function)
        .into_iter()
        .map(|(line, callee_name)| DeferInLoopRef { line, callee_name })
        .collect()
}

/// Go `go` statements launched inside a loop body that has no visible
/// concurrency bound, for `goroutine_in_unbounded_loop` (issue #190).
///
/// This can't reuse `matches_in_loops`: the suppression is scoped to a
/// *specific* loop, so the walk has to know which loop it's inside and
/// whether that loop (or any loop enclosing it) carries a bound, rather
/// than tracking a single "am I in a loop" boolean.
///
/// `is_bound` classifies a node as a recognized concurrency-bounding
/// operation -- for Go, a channel send or receive, which is the acquire
/// half of the standard semaphore/worker-pool idiom. A loop counts as
/// bounded if such a node appears anywhere in its body **outside** any
/// `go` statement: a channel operation *inside* the launched goroutine
/// (`go func() { results <- work() }()`) is the goroutine doing its own
/// work, not the loop throttling how many of them exist, so scanning
/// into launch subtrees would suppress exactly the case this marker is
/// for. `sync.WaitGroup` deliberately doesn't count -- it bounds
/// *completion* tracking, not *concurrency*, so a `wg.Add`/`wg.Done`
/// loop stays flagged.
///
/// `launch_callee` returns the launched call's name for a launch node
/// and `None` for everything else; its `Some`-ness is also what marks a
/// subtree as a launch to skip during the bound scan.
pub fn unbounded_goroutines_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    is_bound: impl Fn(Node) -> bool,
    launch_callee: impl Fn(Node) -> Option<String>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<GoroutineInUnboundedLoopRef> {
    // The four classifiers travel together through both recursions, so
    // they're bundled rather than threaded as four separate parameters.
    struct Scan<'f> {
        is_loop: &'f dyn Fn(Node) -> bool,
        is_bound: &'f dyn Fn(Node) -> bool,
        launch_callee: &'f dyn Fn(Node) -> Option<String>,
        is_nested_function: &'f dyn Fn(Node) -> bool,
    }

    fn has_bound(node: Node, scan: &Scan) -> bool {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if (scan.is_nested_function)(child) || (scan.launch_callee)(child).is_some() {
                continue;
            }
            if (scan.is_bound)(child) || has_bound(child, scan) {
                return true;
            }
        }
        false
    }

    fn walk(node: Node, in_loop: bool, bounded: bool, scan: &Scan, out: &mut Vec<(usize, String)>) {
        if in_loop && !bounded {
            if let Some(name) = (scan.launch_callee)(node) {
                out.push((node.start_position().row + 1, name));
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if (scan.is_nested_function)(child) {
                continue;
            }
            let child_is_loop = (scan.is_loop)(child);
            // Once any enclosing loop is bounded, everything inside it
            // stays suppressed -- an inner loop can't un-bound the
            // semaphore the outer one already acquired.
            let child_bounded = bounded || (child_is_loop && has_bound(child, scan));
            walk(child, in_loop || child_is_loop, child_bounded, scan, out);
        }
    }

    let scan = Scan {
        is_loop: &is_loop,
        is_bound: &is_bound,
        launch_callee: &launch_callee,
        is_nested_function: &is_nested_function,
    };
    let mut out = Vec::new();
    walk(body, false, false, &scan, &mut out);
    out.into_iter()
        .map(|(line, callee_name)| GoroutineInUnboundedLoopRef { line, callee_name })
        .collect()
}

/// Blocking synchronous calls found anywhere in an async function's
/// body, for `blocking_sync_in_async` (issue #184). The caller decides
/// whether the enclosing function is async; this just finds the calls.
pub fn blocking_calls_in_async(
    body: Node,
    blocking_callee: impl Fn(Node) -> Option<String>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<BlockingSyncInAsyncRef> {
    matches_in_body(body, blocking_callee, is_nested_function)
        .into_iter()
        .map(|(line, callee_name)| BlockingSyncInAsyncRef { line, callee_name })
        .collect()
}

/// `pandas.concat` calls found inside a loop body, for
/// `pd_concat_in_loop` (issue #192): each call copies the whole growing
/// DataFrame, making the loop quadratic in the number of rows.
pub fn pd_concats_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    call_callee: impl Fn(Node) -> Option<String>,
    is_pd_concat_call: impl Fn(&str) -> bool,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<PdConcatInLoopRef> {
    matches_in_loops(
        body,
        is_loop,
        |n| call_callee(n).filter(|name| is_pd_concat_call(name)),
        is_nested_function,
    )
    .into_iter()
    .map(|(line, callee_name)| PdConcatInLoopRef { line, callee_name })
    .collect()
}

/// Awaited async calls found inside a loop body, for
/// `serial_await_in_loop` (issue #181): each iteration blocks on the
/// previous one instead of the whole batch running concurrently.
/// `awaited_callee` is expected to already exclude awaits of the
/// concurrency combinators that *are* the fix (`Promise.all`/
/// `join_all`/`asyncio.gather`), since awaiting one inside a loop is
/// the chunked-concurrency shape rather than the serial one.
pub fn serial_awaits_in_loops(
    body: Node,
    is_loop: impl Fn(Node) -> bool,
    awaited_callee: impl Fn(Node) -> Option<String>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<SerialAwaitInLoopRef> {
    matches_in_loops(body, is_loop, awaited_callee, is_nested_function)
        .into_iter()
        .map(|(line, callee_name)| SerialAwaitInLoopRef { line, callee_name })
        .collect()
}

/// Inner loops iterating the same collection as an enclosing loop, for
/// `nested_loop_quadratic` (issue #187) -- the accidental all-pairs
/// O(n^2) scan. Walks carrying a stack of enclosing loops' normalized
/// iterable names and reports a loop whose own iterable is already on
/// that stack.
///
/// Unlike its sibling walks above this takes no separate `is_loop`
/// predicate: `loop_iterable` already node-kind-checks for its
/// language's `for`-loop form, and only a `for` loop has an iterable
/// expression to compare at all (a `while`/`loop` yields `None` and is
/// simply transparent here). A second classifier would be redundant and
/// could silently disagree with the first.
pub fn quadratic_loop_nestings(
    body: Node,
    loop_iterable: impl Fn(Node) -> Option<String>,
    is_nested_function: impl Fn(Node) -> bool,
) -> Vec<NestedLoopQuadraticRef> {
    fn walk(
        node: Node,
        enclosing: &mut Vec<String>,
        loop_iterable: &dyn Fn(Node) -> Option<String>,
        is_nested_function: &dyn Fn(Node) -> bool,
        out: &mut Vec<NestedLoopQuadraticRef>,
    ) {
        let mut pushed = false;
        if let Some(iterable) = loop_iterable(node) {
            if enclosing.contains(&iterable) {
                out.push(NestedLoopQuadraticRef {
                    line: node.start_position().row + 1,
                    iterable: iterable.clone(),
                });
            }
            enclosing.push(iterable);
            pushed = true;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_nested_function(child) {
                continue;
            }
            walk(child, enclosing, loop_iterable, is_nested_function, out);
        }
        if pushed {
            enclosing.pop();
        }
    }
    let mut out = Vec::new();
    walk(
        body,
        &mut Vec::new(),
        &loop_iterable,
        &is_nested_function,
        &mut out,
    );
    out
}

/// Best-effort parameter count: the number of named children of a
/// parameter-list node (may include `self`/`cls`).
pub fn count_params(params: Option<Node>) -> usize {
    params.map(|p| p.named_child_count()).unwrap_or(0)
}

/// Number of declared parameters whose type resolves to a bare primitive.
/// `param_type` extracts a parameter node's declared type as source text
/// (returning `None` for parameters this language/shape doesn't carry a
/// type for, e.g. Rust's `self`); `is_primitive_type` classifies that text.
pub fn primitive_param_count(
    params: Option<Node>,
    param_type: impl Fn(Node) -> Option<String>,
    is_primitive_type: impl Fn(&str) -> bool,
) -> usize {
    let Some(params) = params else {
        return 0;
    };
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter_map(param_type)
        .filter(|t| is_primitive_type(t.as_str()))
        .count()
}

/// Hash of the body's whitespace-normalized text, for best-effort
/// duplicate-code detection. Returns `None` for bodies too short to be a
/// meaningful signal (see `MIN_DUPLICATE_LINES`).
pub fn body_hash(body: Node, source: &str) -> Option<u64> {
    let lines = body
        .end_position()
        .row
        .saturating_sub(body.start_position().row)
        + 1;
    if lines < MIN_DUPLICATE_LINES {
        return None;
    }
    let normalized: String = source[body.byte_range()]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    Some(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_cartesian_join_flags_comma_join_without_predicate() {
        let tables = sql_cartesian_join_tables("SELECT * FROM orders, customers").unwrap();
        assert_eq!(tables, vec!["orders", "customers"]);
    }

    #[test]
    fn sql_cartesian_join_accepts_a_proper_where_join() {
        assert!(sql_cartesian_join_tables(
            "SELECT * FROM orders o, customers c WHERE o.customer_id = c.id"
        )
        .is_none());
    }

    #[test]
    fn sql_cartesian_join_ignores_explicit_join_syntax() {
        assert!(sql_cartesian_join_tables(
            "SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id"
        )
        .is_none());
    }

    #[test]
    fn sql_cartesian_join_needs_a_predicate_per_extra_table() {
        let tables =
            sql_cartesian_join_tables("SELECT * FROM a, b, c WHERE a.id = b.a_id").unwrap();
        assert_eq!(tables, vec!["a", "b", "c"]);
    }

    #[test]
    fn sql_cartesian_join_ignores_a_plain_column_filter() {
        let tables =
            sql_cartesian_join_tables("SELECT * FROM orders o, customers c WHERE o.status = 1")
                .unwrap();
        assert_eq!(tables, vec!["orders", "customers"]);
    }

    #[test]
    fn sql_cartesian_join_ignores_single_table_queries() {
        assert!(sql_cartesian_join_tables("SELECT * FROM orders WHERE id = 1").is_none());
    }

    #[test]
    fn sql_cartesian_join_ignores_non_sql_text() {
        assert!(sql_cartesian_join_tables("just a normal string, with commas").is_none());
    }
}
