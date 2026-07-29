//! Deterministic refactor-candidate detection: reads signals that
//! already exist in `repowise-graph` and `repowise-health` -- file-level
//! import cycles, god classes, low-cohesion classes, duplicate/near-
//! duplicate functions -- and turns each into a [`RefactorCandidate`]:
//! the structural problem, the files/symbols it involves, and a
//! rationale grounded in the measured signal that flagged it.
//!
//! # Report-only, by design
//!
//! This is the deterministic half of issue #304's refactoring layer.
//! The other half -- generating a diff that implements a plan -- turned
//! out to be a genuinely open, needs-a-human question (should this port
//! ever write to source at all, and under what supervision), so it was
//! split off as its own issue rather than resolved by default here.
//! **Nothing in this crate proposes an edit, drafts a diff, or touches a
//! file on disk.** A candidate names the WHAT and the WHERE; the HOW --
//! actually re-arranging the code -- is left to whoever reads the
//! report. `repowise health`'s own worst files in this very repo score
//! 0.0, which is exactly the kind of finding an unsupervised rewriter
//! would be a bad idea to point at itself.
//!
//! # Synthesis, not new detection
//!
//! Every underlying signal already existed and is independently useful
//! on its own -- `repowise health`'s god-class/low-cohesion/duplicate-
//! code markers, `repowise-graph`'s dependency edges. What was missing
//! was consuming them *together* as refactor candidates rather than as
//! separate scores nobody had connected back to an action. Issue #304's
//! "graph-aware analysis to produce plans worth acting on" is this
//! synthesis step, not a new detector underneath it.
//!
//! The one genuinely new piece is
//! [`repowise_graph::RepoGraph::file_import_cycles`] -- nothing computed
//! single-repo import cycles before this; only `repowise-workspace`'s
//! cross-repo cycle detection existed.
//!
//! # Deterministic top to bottom
//!
//! No LLM involvement anywhere in this crate. Every field on every
//! candidate traces back to a graph edge or a health marker that was
//! already computed the same way every time -- there is nothing here
//! for a model to get confidently wrong, and nothing to anchor-check the
//! way `repowise-adr::inferred` has to for its one non-deterministic
//! source.
//!
//! # This list is uncapped; callers must cap it
//!
//! Running this against this port's own workspace surfaced why: on a
//! multi-crate Rust codebase this size, near-duplicate function pairs
//! alone number in the thousands -- mostly structurally-similar test
//! fixtures across ~20 crates' test suites, `repowise health`'s own
//! `near-duplicate-code` marker is already this noisy repo-wide (over
//! 12,000 findings), it's just never surfaced as an uncapped flat list
//! anywhere else; every existing consumer folds it into a per-file
//! score instead. This crate is the first to expose duplicate pairs
//! individually, so it's the first place the raw count becomes visible.
//!
//! [`find_refactor_candidates`] deliberately does **not** cap its own
//! output -- capping is a display concern, and belongs to the caller
//! (the CLI's `--limit`, the MCP tool's `limit`/`*_total` fields), the
//! same "the detector returns everything, the surface caps it" split
//! `repowise_health::find_dead_code`/`get_dead_code` already uses. What
//! this module *does* guarantee is that duplicate candidates come back
//! ranked strongest-first (exact matches, then near-duplicates by
//! descending overlap) so that whatever cap a caller applies keeps the
//! signal rather than an arbitrary file-order slice.

use repowise_core::RepoIndex;
use repowise_graph::RepoGraph;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What kind of structural problem a candidate names.
#[derive(Debug, Clone, PartialEq)]
pub enum RefactorKind {
    /// A -> B -> ... -> A file-level import cycle (including a file
    /// that imports itself).
    BreakImportCycle,
    /// A class/struct with more methods than
    /// [`repowise_health::GOD_CLASS_METHODS`].
    SplitGodClass { method_count: usize },
    /// A class whose field-touching methods split into 2+ disjoint
    /// groups -- see `repowise_health::find_low_cohesion`'s own doc for
    /// what "field-touching" excludes.
    SplitByCohesion {
        components: usize,
        tracked_methods: usize,
    },
    /// Two functions/methods whose bodies are identical (`overlap ==
    /// 1.0`) or measured as substantially similar by
    /// `repowise_health::find_near_duplicates`.
    ExtractDuplicate { overlap: f64 },
}

impl RefactorKind {
    pub fn label(&self) -> &'static str {
        match self {
            RefactorKind::BreakImportCycle => "break-import-cycle",
            RefactorKind::SplitGodClass { .. } => "split-god-class",
            RefactorKind::SplitByCohesion { .. } => "split-by-cohesion",
            RefactorKind::ExtractDuplicate { .. } => "extract-duplicate",
        }
    }
}

/// One deterministic refactor candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct RefactorCandidate {
    /// Stable, content-derived -- built from the files/symbols involved,
    /// never from list position, so the same repo state always produces
    /// the same id.
    pub id: String,
    pub kind: RefactorKind,
    pub title: String,
    /// Plain-language explanation, always traceable to a specific
    /// measured number (a method count, a component count, an overlap
    /// ratio) rather than a vague judgment call.
    pub rationale: String,
    /// Files this candidate concerns, repo-relative, sorted and
    /// deduplicated. Never empty.
    pub files: Vec<String>,
    /// Symbol names involved (class or function names). Empty for a
    /// pure import-cycle candidate, which is file-scoped rather than
    /// symbol-scoped.
    pub symbols: Vec<String>,
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Find every deterministic refactor candidate in `index`.
///
/// Four sources, each independent -- one source finding nothing doesn't
/// affect the others, the same "no docs/adr/ -> that source is just
/// empty" convention `repowise-adr::mine` uses. Order is stable:
/// cycles, then god classes, then low-cohesion classes, then duplicate
/// pairs, each internally sorted worst-first by its own source.
pub fn find_refactor_candidates(index: &RepoIndex, graph: &RepoGraph) -> Vec<RefactorCandidate> {
    let mut out = Vec::new();
    out.extend(cycle_candidates(index, graph));
    out.extend(god_class_candidates(index));
    out.extend(cohesion_candidates(index));
    out.extend(duplicate_candidates(index));
    out
}

fn cycle_candidates(index: &RepoIndex, graph: &RepoGraph) -> Vec<RefactorCandidate> {
    let mut cycles = graph.file_import_cycles();
    // Deterministic output: `file_import_cycles` doesn't promise an
    // order (SCC discovery order isn't part of its contract), so pin
    // both the outer list and each group's own file order here rather
    // than let it vary by petgraph's internal traversal.
    for group in &mut cycles {
        group.sort();
    }
    cycles.sort();

    cycles
        .into_iter()
        .map(|files| {
            let rel: Vec<String> = files.iter().map(|f| relative(&index.root, f)).collect();
            let title = if rel.len() == 1 {
                format!("{} imports itself", rel[0])
            } else {
                format!("Import cycle across {} files", rel.len())
            };
            RefactorCandidate {
                id: format!("cycle:{}", rel.join(",")),
                kind: RefactorKind::BreakImportCycle,
                title,
                rationale: format!(
                    "These files import each other in a cycle: {}. Breaking it usually means \
                     moving the shared piece each side depends on into a third file neither \
                     of them needs to import back.",
                    rel.join(" -> ")
                ),
                files: rel,
                symbols: Vec::new(),
            }
        })
        .collect()
}

fn god_class_candidates(index: &RepoIndex) -> Vec<RefactorCandidate> {
    repowise_health::find_god_classes(index)
        .into_iter()
        .map(|c| {
            let file = relative(&index.root, &c.file);
            RefactorCandidate {
                id: format!("god-class:{file}:{}", c.class),
                kind: RefactorKind::SplitGodClass {
                    method_count: c.method_count,
                },
                title: format!("{} has {} methods", c.class, c.method_count),
                rationale: format!(
                    "{} methods on `{}` (> {}) is a plausible sign it's doing more than one \
                     job -- worth checking whether its methods split into groups that could \
                     become separate types.",
                    c.method_count,
                    c.class,
                    repowise_health::GOD_CLASS_METHODS
                ),
                files: vec![file],
                symbols: vec![c.class],
            }
        })
        .collect()
}

fn cohesion_candidates(index: &RepoIndex) -> Vec<RefactorCandidate> {
    repowise_health::find_low_cohesion(index)
        .into_iter()
        .map(|c| {
            let file = relative(&index.root, &c.file);
            RefactorCandidate {
                id: format!("low-cohesion:{file}:{}", c.class),
                kind: RefactorKind::SplitByCohesion {
                    components: c.components,
                    tracked_methods: c.tracked_methods,
                },
                title: format!(
                    "{} splits into {} field-access groups",
                    c.class, c.components
                ),
                rationale: format!(
                    "Of {} field-touching methods on `{}`, none in one group touch a field \
                     also touched by a method in another group ({} disjoint groups). That's \
                     evidence the class's own fields don't tie its methods together -- a \
                     structural candidate for splitting along those group lines, not a style \
                     preference.",
                    c.tracked_methods, c.class, c.components
                ),
                files: vec![file],
                symbols: vec![c.class],
            }
        })
        .collect()
}

/// Exact-duplicate pairs (identical `body_hash`), the same grouping
/// `repowise_health`'s own `DuplicateCode` marker uses internally --
/// re-derived here as structured pairs rather than consumed from
/// `analyze()`'s `Finding.detail` strings, which only list the other
/// symbols' *names* in prose. A refactor candidate needs the other
/// symbol's file too, to say where the extraction would go.
fn exact_duplicate_pairs(
    index: &RepoIndex,
) -> Vec<(PathBuf, String, usize, PathBuf, String, usize)> {
    use std::collections::HashMap;
    let mut groups: HashMap<u64, Vec<(&PathBuf, &str, usize)>> = HashMap::new();
    for file in &index.files {
        for sym in &file.symbols {
            if let Some(hash) = sym.body_hash {
                groups
                    .entry(hash)
                    .or_default()
                    .push((&file.path, &sym.name, sym.start_line));
            }
        }
    }
    let mut pairs = Vec::new();
    for group in groups.values() {
        if group.len() < 2 {
            continue;
        }
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                let (fa, na, la) = group[i];
                let (fb, nb, lb) = group[j];
                pairs.push((
                    fa.clone(),
                    na.to_string(),
                    la,
                    fb.clone(),
                    nb.to_string(),
                    lb,
                ));
            }
        }
    }
    pairs
}

fn duplicate_candidates(index: &RepoIndex) -> Vec<RefactorCandidate> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();

    for (file, symbol, _line, other_file, other_symbol, _other_line) in exact_duplicate_pairs(index)
    {
        push_duplicate_candidate(
            index,
            &mut seen,
            &mut out,
            &file,
            &symbol,
            &other_file,
            &other_symbol,
            1.0,
        );
    }

    // `find_near_duplicates` excludes anything already caught by the
    // exact-hash check above, and reports each pair from both symbols'
    // side -- `seen` collapses that back to one candidate per pair.
    for c in repowise_health::find_near_duplicates(index) {
        push_duplicate_candidate(
            index,
            &mut seen,
            &mut out,
            &c.file,
            &c.symbol,
            &c.other_file,
            &c.other_symbol,
            c.overlap_ratio,
        );
    }

    // Exact duplicates (overlap 1.0) and the strongest near-duplicates
    // first. Verified against this port's own codebase: near-duplicate
    // pairs alone number in the thousands on a multi-crate Rust
    // workspace this size (mostly structurally-similar test fixtures),
    // which is exactly why callers must cap this list -- but a cap only
    // does its job if what survives it is the strongest signal, not an
    // arbitrary slice in file/line order.
    out.sort_by(|a, b| {
        let overlap_a = match a.kind {
            RefactorKind::ExtractDuplicate { overlap } => overlap,
            _ => 0.0,
        };
        let overlap_b = match b.kind {
            RefactorKind::ExtractDuplicate { overlap } => overlap,
            _ => 0.0,
        };
        overlap_b
            .partial_cmp(&overlap_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.id.cmp(&b.id))
    });
    out
}

#[allow(clippy::too_many_arguments)]
fn push_duplicate_candidate(
    index: &RepoIndex,
    seen: &mut HashSet<(String, String)>,
    out: &mut Vec<RefactorCandidate>,
    file: &Path,
    symbol: &str,
    other_file: &Path,
    other_symbol: &str,
    overlap: f64,
) {
    let rel = relative(&index.root, file);
    let other_rel = relative(&index.root, other_file);

    // Canonicalize by sorting the pair -- `find_near_duplicates` reports
    // (a, b) and (b, a) as separate entries, and without this a genuine
    // near-duplicate pair would be reported twice.
    let mut key = [
        (rel.clone(), symbol.to_string()),
        (other_rel.clone(), other_symbol.to_string()),
    ];
    key.sort();
    let canonical = (
        format!("{}::{}", key[0].0, key[0].1),
        format!("{}::{}", key[1].0, key[1].1),
    );
    if !seen.insert(canonical.clone()) {
        return;
    }

    let exact = overlap >= 1.0;
    let mut files = vec![rel, other_rel];
    files.sort();
    files.dedup();
    let mut symbols = vec![symbol.to_string(), other_symbol.to_string()];
    symbols.sort();

    out.push(RefactorCandidate {
        id: format!("extract-duplicate:{}:{}", canonical.0, canonical.1),
        kind: RefactorKind::ExtractDuplicate { overlap },
        title: format!("{symbol} and {other_symbol} look like the same function"),
        rationale: if exact {
            format!(
                "`{symbol}` and `{other_symbol}` have identical bodies -- a strong signal one \
                 should call the other, or both should call a shared extraction."
            )
        } else {
            format!(
                "`{symbol}` and `{other_symbol}` share {:.0}% of their normalized text -- \
                 similar enough to be worth checking whether they're the same logic \
                 duplicated rather than two things that happen to look alike.",
                overlap * 100.0
            )
        },
        files,
        symbols,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn build(root: &Path) -> (RepoIndex, RepoGraph) {
        let discovered = repowise_core::discover_files(root).unwrap();
        let mut files = Vec::new();
        let mut other_files = 0;
        for entry in discovered {
            if matches!(entry.language, repowise_core::Language::Other) {
                other_files += 1;
                continue;
            }
            let source = fs::read_to_string(&entry.path).unwrap();
            match repowise_parser::parse_file(&entry.path, entry.language, &source).unwrap() {
                Some(record) => files.push(record),
                None => other_files += 1,
            }
        }
        let index = RepoIndex {
            root: root.to_path_buf(),
            files,
            other_files,
            indexed_commit: None,
        };
        let graph = RepoGraph::build(&index);
        (index, graph)
    }

    #[test]
    fn an_import_cycle_becomes_a_break_import_cycle_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/__init__.py"), "").unwrap();
        fs::write(
            root.join("pkg/a.py"),
            "from pkg.b import b_helper\n\ndef a_helper():\n    return b_helper()\n",
        )
        .unwrap();
        fs::write(
            root.join("pkg/b.py"),
            "from pkg.a import a_helper\n\ndef b_helper():\n    return 1\n",
        )
        .unwrap();

        let (index, graph) = build(&root);
        let candidates = find_refactor_candidates(&index, &graph);

        let cycles: Vec<&RefactorCandidate> = candidates
            .iter()
            .filter(|c| matches!(c.kind, RefactorKind::BreakImportCycle))
            .collect();
        assert_eq!(cycles.len(), 1, "{candidates:?}");
        assert_eq!(cycles[0].files.len(), 2);
        assert!(cycles[0].symbols.is_empty(), "a cycle is file-scoped");
        assert!(
            cycles[0].rationale.contains("->"),
            "{}",
            cycles[0].rationale
        );
    }

    #[test]
    fn an_acyclic_repo_reports_no_cycle_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("a.py"), "def a():\n    return 1\n").unwrap();

        let (index, graph) = build(&root);
        let candidates = find_refactor_candidates(&index, &graph);
        assert!(candidates
            .iter()
            .all(|c| !matches!(c.kind, RefactorKind::BreakImportCycle)));
    }

    #[test]
    fn a_god_class_becomes_a_split_god_class_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut body = String::from("class Big:\n");
        for i in 0..(repowise_health::GOD_CLASS_METHODS + 1) {
            body.push_str(&format!("    def m{i}(self):\n        return {i}\n"));
        }
        fs::write(root.join("big.py"), body).unwrap();

        let (index, graph) = build(&root);
        let candidates = find_refactor_candidates(&index, &graph);

        let god = candidates
            .iter()
            .find(|c| matches!(c.kind, RefactorKind::SplitGodClass { .. }))
            .expect("Big must be flagged");
        assert_eq!(god.symbols, vec!["Big".to_string()]);
        assert!(god
            .rationale
            .contains(&(repowise_health::GOD_CLASS_METHODS + 1).to_string()));
    }

    /// A duplicate pair reported by `find_near_duplicates` from both
    /// symbols' side must become exactly one candidate, not two.
    #[test]
    fn a_duplicate_pair_is_reported_once_not_twice() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(
            root.join("a.py"),
            "def compute_total(items):\n    total = 0\n    for x in items:\n        total += x\n    return total\n",
        )
        .unwrap();
        fs::write(
            root.join("b.py"),
            "def compute_total(items):\n    total = 0\n    for x in items:\n        total += x\n    return total\n",
        )
        .unwrap();

        let (index, graph) = build(&root);
        let candidates = find_refactor_candidates(&index, &graph);

        let dups: Vec<&RefactorCandidate> = candidates
            .iter()
            .filter(|c| matches!(c.kind, RefactorKind::ExtractDuplicate { .. }))
            .collect();
        assert_eq!(dups.len(), 1, "{candidates:?}");
        assert_eq!(
            dups[0].kind,
            RefactorKind::ExtractDuplicate { overlap: 1.0 },
            "identical bodies must report overlap 1.0"
        );
        assert_eq!(dups[0].files.len(), 2);
    }

    /// Found by running `repowise refactor` against this port's own
    /// workspace: near-duplicate pairs alone numbered in the thousands
    /// (mostly structurally-similar test fixtures across ~20 crates),
    /// which any capped display must survive by keeping the strongest
    /// signal, not an arbitrary slice. Exact duplicates (overlap 1.0)
    /// must sort before a near-duplicate pair at a lower ratio.
    #[test]
    fn duplicate_candidates_rank_exact_matches_before_near_matches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // An exact duplicate: identical body, different names/files.
        fs::write(
            root.join("a.py"),
            "def compute_total(items):\n    total = 0\n    for x in items:\n        total += x\n    return total\n",
        )
        .unwrap();
        fs::write(
            root.join("b.py"),
            "def compute_total(items):\n    total = 0\n    for x in items:\n        total += x\n    return total\n",
        )
        .unwrap();
        // A near-duplicate: same structure, renamed variable and a
        // tweaked constant -- the exact pair used by
        // `repowise-health::near_duplicate`'s own test fixture, so this
        // is a known below-1.0 overlap ratio, not a guessed one.
        fs::write(
            root.join("widgets.rs"),
            "fn process_widget(count: i32) -> i32 {\n    let mut total = 0;\n    for i in 0..count {\n        total += i * 2;\n        if total > 1000 {\n            total -= 1000;\n        }\n    }\n    total + 1\n}\n\nfn process_gadget(count: i32) -> i32 {\n    let mut sum = 0;\n    for i in 0..count {\n        sum += i * 2;\n        if sum > 1000 {\n            sum -= 1000;\n        }\n    }\n    sum + 2\n}\n",
        )
        .unwrap();

        let (index, graph) = build(&root);
        let candidates = find_refactor_candidates(&index, &graph);

        let dups: Vec<&RefactorCandidate> = candidates
            .iter()
            .filter(|c| matches!(c.kind, RefactorKind::ExtractDuplicate { .. }))
            .collect();
        assert_eq!(dups.len(), 2, "{candidates:?}");
        let overlaps: Vec<f64> = dups
            .iter()
            .map(|c| match c.kind {
                RefactorKind::ExtractDuplicate { overlap } => overlap,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            overlaps[0], 1.0,
            "the exact match must rank first: {overlaps:?}"
        );
        assert!(
            overlaps[1] < 1.0,
            "the near-duplicate must rank after the exact match: {overlaps:?}"
        );
    }

    #[test]
    fn every_candidate_id_is_stable_across_repeated_calls() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // 4+ line bodies -- shorter than `MIN_DUPLICATE_LINES` never
        // gets a `body_hash` at all, so a candidate wouldn't fire.
        fs::write(
            root.join("a.py"),
            "def compute_total(items):\n    total = 0\n    for x in items:\n        total += x\n    return total\n",
        )
        .unwrap();
        fs::write(
            root.join("b.py"),
            "def compute_total(items):\n    total = 0\n    for x in items:\n        total += x\n    return total\n",
        )
        .unwrap();

        let (index, graph) = build(&root);
        let first: Vec<String> = find_refactor_candidates(&index, &graph)
            .into_iter()
            .map(|c| c.id)
            .collect();
        let second: Vec<String> = find_refactor_candidates(&index, &graph)
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    #[test]
    fn a_repo_with_no_structural_problems_reports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("clean.py"), "def solo():\n    return 1\n").unwrap();

        let (index, graph) = build(&root);
        assert!(find_refactor_candidates(&index, &graph).is_empty());
    }

    #[test]
    fn kind_labels_are_all_distinct() {
        let kinds = [
            RefactorKind::BreakImportCycle,
            RefactorKind::SplitGodClass { method_count: 1 },
            RefactorKind::SplitByCohesion {
                components: 2,
                tracked_methods: 4,
            },
            RefactorKind::ExtractDuplicate { overlap: 1.0 },
        ];
        let labels: HashSet<&str> = kinds.iter().map(|k| k.label()).collect();
        assert_eq!(labels.len(), kinds.len());
    }
}
