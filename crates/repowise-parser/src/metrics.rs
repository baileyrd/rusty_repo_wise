//! Language-agnostic per-symbol metrics computed directly from the AST:
//! cyclomatic complexity, max nesting depth, a "bumpy road" nested-block
//! count, parameter count, and a duplicate-code body hash. These feed
//! `repowise-health`'s deterministic scoring.

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
