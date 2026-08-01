//! Louvain modularity-based community detection over an unweighted,
//! undirected graph -- issue #352's Map sub-view: "the detected
//! communities within the dependency graph laid out on a module map,
//! with sizing proportional to code volume in each component."
//! Reading upstream's own `docs/architecture/graph-algorithms.md`
//! confirmed this is what "Map" means (not a manually curated layering
//! -- see that module's own note on a separate, unrelated "curated
//! layer" concept used only for optional Structurizr DSL export).
//!
//! Standard multi-level Louvain: repeated local-moving passes (move
//! each node into whichever neighboring community most increases
//! modularity) followed by aggregation (collapse each community into a
//! single super-node and repeat), until no aggregation step reduces the
//! node count any further. See Blondel et al., "Fast unfolding of
//! communities in large networks" (2008) for the algorithm this
//! follows; nothing here is novel.
//!
//! Generic over the node type so it can run over files, symbols, or
//! anything else this port might someday want communities over --
//! `T: Ord` is required only so the final grouping is fully
//! deterministic (sorted by community size, then by member identity),
//! not because the algorithm itself needs an ordering.

use std::collections::HashMap;
use std::hash::Hash;

/// Safety bound on local-moving passes within one aggregation level --
/// real graphs converge in single digits of passes; this is a backstop
/// against a pathological input oscillating forever.
const MAX_PASSES_PER_LEVEL: usize = 100;
/// Safety bound on aggregation levels. Each successful level strictly
/// reduces the node count, so this is a generous backstop, not a
/// typical depth.
const MAX_LEVELS: usize = 50;

/// Detect communities among `nodes` connected by `edges` (each edge an
/// unordered pair; a duplicate edge or the reverse of another edge adds
/// weight rather than being ignored, so e.g. two files that import each
/// other are more tightly bound than two that import only one way).
/// Self-edges (`a == b`) and edges naming a node not in `nodes` are
/// ignored. Returns one `Vec<T>` per detected community, communities
/// sorted largest-first (ties broken by their smallest member, for
/// determinism); a node with no edges at all ends up alone in its own
/// community.
pub fn detect_communities<T>(nodes: &[T], edges: &[(T, T)]) -> Vec<Vec<T>>
where
    T: Eq + Hash + Clone + Ord,
{
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let index_of: HashMap<T, usize> = nodes
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, node)| (node, i))
        .collect();

    let mut weight_map: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n];
    for (a, b) in edges {
        let (Some(&ia), Some(&ib)) = (index_of.get(a), index_of.get(b)) else {
            continue;
        };
        if ia == ib {
            continue;
        }
        *weight_map[ia].entry(ib).or_insert(0.0) += 1.0;
        *weight_map[ib].entry(ia).or_insert(0.0) += 1.0;
    }
    let mut adj: Vec<Vec<(usize, f64)>> = weight_map
        .into_iter()
        .map(|m| m.into_iter().collect())
        .collect();

    let total_weight: f64 = adj.iter().flatten().map(|&(_, w)| w).sum::<f64>() / 2.0;
    // No edges at all: every node is its own community, no modularity
    // computation is meaningful (division by a zero total weight).
    if total_weight == 0.0 {
        return nodes.iter().map(|node| vec![node.clone()]).collect();
    }

    let mut self_loops: Vec<f64> = vec![0.0; n];
    // `membership[i]` maps original node `i` to its current community id
    // at whatever level aggregation has reached so far.
    let mut membership: Vec<usize> = (0..n).collect();

    for _level in 0..MAX_LEVELS {
        let local_comm = local_moving(&adj, &self_loops, total_weight);
        let (renumbered, k) = renumber(&local_comm);
        if k == adj.len() {
            // Local moving made no progress at all this level (every
            // node stayed its own community) -- further levels can't
            // help either.
            break;
        }
        for m in membership.iter_mut() {
            *m = renumbered[local_comm[*m]];
        }
        if k <= 1 {
            break;
        }
        let (new_adj, new_self_loops) = aggregate(&adj, &self_loops, &renumbered, k);
        adj = new_adj;
        self_loops = new_self_loops;
    }

    let mut groups: HashMap<usize, Vec<T>> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        groups.entry(membership[i]).or_default().push(node.clone());
    }
    let mut out: Vec<Vec<T>> = groups.into_values().collect();
    for group in &mut out {
        group.sort();
    }
    out.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then_with(|| a.first().cmp(&b.first()))
    });
    out
}

/// One node's current community, plus every neighboring community's
/// total weighted degree at the point local moving starts -- iterated
/// to a fixed point (or `MAX_PASSES_PER_LEVEL`).
fn local_moving(adj: &[Vec<(usize, f64)>], self_loops: &[f64], m: f64) -> Vec<usize> {
    let n = adj.len();
    let degree: Vec<f64> = (0..n)
        .map(|i| {
            let external: f64 = adj[i].iter().map(|&(_, w)| w).sum();
            external + 2.0 * self_loops[i]
        })
        .collect();

    let mut community: Vec<usize> = (0..n).collect();
    let mut community_degree: Vec<f64> = degree.clone();

    for _pass in 0..MAX_PASSES_PER_LEVEL {
        let mut moved = false;
        for i in 0..n {
            let ci = community[i];
            community_degree[ci] -= degree[i];

            let mut neighbor_weights: HashMap<usize, f64> = HashMap::new();
            for &(j, w) in &adj[i] {
                *neighbor_weights.entry(community[j]).or_insert(0.0) += w;
            }
            // The node's own (post-removal) community is always a
            // candidate, even with zero weight to it, so "stay put" is
            // never worse than the arithmetic makes some other
            // zero-weight community look.
            neighbor_weights.entry(ci).or_insert(0.0);

            let mut candidates: Vec<(usize, f64)> = neighbor_weights.into_iter().collect();
            candidates.sort_by_key(|&(c, _)| c);

            let mut best_c = ci;
            let mut best_gain = f64::NEG_INFINITY;
            for (c, k_in) in candidates {
                let gain = k_in - (community_degree[c] * degree[i]) / (2.0 * m);
                if gain > best_gain {
                    best_gain = gain;
                    best_c = c;
                }
            }

            community_degree[best_c] += degree[i];
            if best_c != ci {
                community[i] = best_c;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    community
}

/// Renumber an arbitrary (possibly sparse) community-id assignment to
/// dense `0..k`, in first-seen order -- deterministic given a
/// deterministic `assignment`.
fn renumber(assignment: &[usize]) -> (Vec<usize>, usize) {
    let mut seen: HashMap<usize, usize> = HashMap::new();
    let mut next = 0;
    let mut out = vec![0; assignment.len()];
    for (i, &c) in assignment.iter().enumerate() {
        let id = *seen.entry(c).or_insert_with(|| {
            let v = next;
            next += 1;
            v
        });
        out[i] = id;
    }
    (out, next)
}

/// Collapse `adj`/`self_loops` down to `k` super-nodes per `renumbered`
/// (each original node's new community id), preserving total edge
/// weight -- the invariant multi-level Louvain depends on to keep `m`
/// valid at every level without recomputing it.
fn aggregate(
    adj: &[Vec<(usize, f64)>],
    self_loops: &[f64],
    renumbered: &[usize],
    k: usize,
) -> (Vec<Vec<(usize, f64)>>, Vec<f64>) {
    let mut new_self_loops = vec![0.0; k];
    // Internal-edge weight accumulates once per direction (twice per
    // undirected edge, since `adj` stores both `(i, j)` and `(j, i)`),
    // which is exactly the doubling a self-loop's weight needs to
    // contribute correctly to degree (see `local_moving`'s `2.0 *
    // self_loops[i]`) -- halved back only for the carried-forward part.
    let mut internal_double = vec![0.0; k];
    let mut new_weight_map: Vec<HashMap<usize, f64>> = vec![HashMap::new(); k];

    for i in 0..adj.len() {
        let ci = renumbered[i];
        new_self_loops[ci] += self_loops[i];
        for &(j, w) in &adj[i] {
            let cj = renumbered[j];
            if ci == cj {
                internal_double[ci] += w;
            } else {
                *new_weight_map[ci].entry(cj).or_insert(0.0) += w;
            }
        }
    }
    for c in 0..k {
        new_self_loops[c] += internal_double[c] / 2.0;
    }

    let new_adj = new_weight_map
        .into_iter()
        .map(|m| m.into_iter().collect())
        .collect();
    (new_adj, new_self_loops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_with_no_edges_are_each_their_own_community() {
        let nodes = vec!["a", "b", "c"];
        let communities = detect_communities(&nodes, &[]);
        assert_eq!(communities.len(), 3, "{communities:?}");
        assert!(communities.iter().all(|c| c.len() == 1));
    }

    #[test]
    fn a_single_connected_component_is_one_community() {
        let nodes = vec!["a", "b", "c"];
        let edges = vec![("a", "b"), ("b", "c"), ("a", "c")];
        let communities = detect_communities(&nodes, &edges);
        assert_eq!(communities.len(), 1, "{communities:?}");
        assert_eq!(communities[0].len(), 3);
    }

    /// Two tightly-connected triangles joined by a single bridge edge
    /// -- the canonical toy case every community-detection algorithm is
    /// expected to split correctly: the triangles are communities, the
    /// bridge is not enough to merge them.
    #[test]
    fn two_triangles_joined_by_one_bridge_split_into_two_communities() {
        let nodes = vec!["a1", "a2", "a3", "b1", "b2", "b3"];
        let edges = vec![
            ("a1", "a2"),
            ("a2", "a3"),
            ("a1", "a3"),
            ("b1", "b2"),
            ("b2", "b3"),
            ("b1", "b3"),
            ("a1", "b1"),
        ];
        let communities = detect_communities(&nodes, &edges);
        assert_eq!(communities.len(), 2, "{communities:?}");
        assert_eq!(communities[0].len(), 3);
        assert_eq!(communities[1].len(), 3);
        let community_of = |node: &str| communities.iter().position(|c| c.contains(&node)).unwrap();
        assert_eq!(community_of("a1"), community_of("a2"));
        assert_eq!(community_of("a1"), community_of("a3"));
        assert_eq!(community_of("b1"), community_of("b2"));
        assert_ne!(community_of("a1"), community_of("b1"));
    }

    #[test]
    fn every_original_node_appears_in_exactly_one_community() {
        let nodes: Vec<usize> = (0..12).collect();
        let edges = vec![
            (0, 1),
            (1, 2),
            (0, 2),
            (3, 4),
            (4, 5),
            (3, 5),
            (6, 7),
            (7, 8),
            (6, 8),
            (9, 10),
            (10, 11),
            (9, 11),
            (2, 3),
            (5, 6),
            (8, 9),
        ];
        let communities = detect_communities(&nodes, &edges);
        let mut all: Vec<usize> = communities.into_iter().flatten().collect();
        all.sort();
        assert_eq!(all, (0..12).collect::<Vec<_>>());
    }

    #[test]
    fn result_is_deterministic_across_repeated_runs() {
        let nodes: Vec<usize> = (0..20).collect();
        let edges: Vec<(usize, usize)> = (0..19).map(|i| (i, i + 1)).collect();
        let first = detect_communities(&nodes, &edges);
        let second = detect_communities(&nodes, &edges);
        assert_eq!(first, second);
    }

    #[test]
    fn a_duplicate_edge_direction_does_not_create_a_phantom_community() {
        let nodes = vec!["a", "b"];
        let edges = vec![("a", "b"), ("b", "a")];
        let communities = detect_communities(&nodes, &edges);
        assert_eq!(communities.len(), 1, "{communities:?}");
        assert_eq!(communities[0].len(), 2);
    }
}
