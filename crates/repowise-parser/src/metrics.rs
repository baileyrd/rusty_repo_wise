//! Language-agnostic per-symbol metrics computed directly from the AST:
//! cyclomatic complexity, parameter count, and a duplicate-code body hash.
//! These feed `repowise-health`'s deterministic scoring.

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

/// Best-effort parameter count: the number of named children of a
/// parameter-list node (may include `self`/`cls`).
pub fn count_params(params: Option<Node>) -> usize {
    params.map(|p| p.named_child_count()).unwrap_or(0)
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
