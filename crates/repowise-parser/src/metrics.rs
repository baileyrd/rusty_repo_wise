//! Language-agnostic per-symbol metrics computed directly from the AST:
//! cyclomatic complexity, max nesting depth, a "bumpy road" nested-block
//! count, per-condition boolean-operator counting, parameter count, and
//! a duplicate-code body hash. These feed `repowise-health`'s
//! deterministic scoring.

use repowise_core::{
    ComplexConditionalRef, IoInLoopRef, JsonParseInLoopRef, ListInsertZeroInLoopRef, LockInLoopRef,
    RegexCompileInLoopRef, ResourceConstructionInLoopRef, StringConcatInLoopRef,
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
