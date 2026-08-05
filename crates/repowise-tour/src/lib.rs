//! Builds a *guided tour*: an ordered reading path through a codebase,
//! answering the one question this port could never answer before —
//! "where do I start, and what do I read next?"
//!
//! Everything else here reports on a file you already named
//! (`repowise deps`, `repowise health`, the per-file wiki pages) or
//! ranks files by one metric (`repowise hotspots`,
//! `repowise overview`'s most-depended-on list). None of that is a
//! reading order: a ranking tells you which file matters most, not what
//! to read second so that the third makes sense.
//!
//! # Deterministic, top to bottom
//!
//! No LLM anywhere in this crate — the ordering is derived from the
//! `Imports` edges `repowise-graph` already resolved, and the ranking
//! from counts already in the index. The same commit always produces the
//! same tour. Upstream prior art (Understand-Anything's `tour-builder`)
//! generates its walkthroughs with a model; that buys prose, not
//! ordering, and prose can be layered on afterwards by `repowise-llm`
//! the same opt-in way it already layers summaries onto wiki pages.
//!
//! # Dependencies first, entry points last
//!
//! A tour is ordered so that nothing is introduced before the things it
//! is built out of. Concretely: for an `Imports` edge `A -> B` (A
//! imports B), B is read before A. The tour therefore opens on the
//! foundations — the files that depend on nothing else in the tour — and
//! closes on the entry points that tie them together.
//!
//! The opposite reading (start at `main`, descend) is a legitimate and
//! genuinely different preference, not an oversight; see
//! [`TourOptions::max_steps`]'s sibling discussion in the CLI docs and
//! issue #377 for why only one direction ships first.
//!
//! # A tour is a selection, not the whole repo
//!
//! Ordering every file is not a tour — it is the repo, shuffled. The
//! selection step keeps the most load-bearing files ([`rank_candidates`]),
//! reserves a slice for entry points so the tour reaches something you
//! can actually run ([`select`]), and drops the rest. That is why
//! [`Tour::considered`] reports how many files were in the running: a
//! tour that shows 15 of 900 files must not read as though the repo has
//! 15 files.

use petgraph::algo::kosaraju_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use repowise_core::{FileRecord, RepoIndex};
use repowise_graph::RepoGraph;
use repowise_health::HealthReport;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Default number of steps in a tour. Chosen to be readable in one
/// sitting rather than to cover a repo: past roughly this many files
/// a "tour" stops being something a person actually walks through.
pub const DEFAULT_MAX_STEPS: usize = 15;

/// What structural part a file plays in the tour, derived purely from
/// its resolved `Imports` edges across the *whole* graph (not just the
/// selected subset — a file's role in the repo doesn't change because
/// the tour is short).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepRole {
    /// Nothing imports it, and it imports something: a top-level entry
    /// point (a binary's `main`, a CLI module, a test harness).
    EntryPoint,
    /// Imported by others, imports nothing itself: bedrock the rest of
    /// the codebase is stacked on.
    Foundation,
    /// Both imported by others and importing others — the middle of the
    /// dependency graph, where most real logic lives.
    Connector,
    /// Neither imports nor is imported by any *resolved* edge. Usually
    /// means this port's heuristics couldn't resolve its imports rather
    /// than that the file is genuinely standalone, so it is ranked last
    /// and labelled honestly.
    Isolated,
}

impl StepRole {
    pub fn label(&self) -> &'static str {
        match self {
            StepRole::EntryPoint => "entry-point",
            StepRole::Foundation => "foundation",
            StepRole::Connector => "connector",
            StepRole::Isolated => "isolated",
        }
    }
}

/// One stop on the tour.
#[derive(Debug, Clone)]
pub struct TourStep {
    /// 1-based position in the reading order.
    pub position: usize,
    pub file: PathBuf,
    pub role: StepRole,
    /// Files that import this one (repo-wide, not tour-local).
    pub dependents: usize,
    /// Files this one imports (repo-wide, not tour-local).
    pub dependencies: usize,
    pub symbols: usize,
    pub lines: usize,
    /// This file's health score (0.0–10.0), when a [`HealthReport`] was
    /// supplied. `None` means "not measured", never "healthy".
    pub health: Option<f64>,
    /// Hotspot score (churn × complexity), when hotspot data was
    /// supplied. `None` means "not measured", never "cold" — an
    /// un-versioned checkout has no churn data at all.
    pub hotspot: Option<usize>,
    /// The other files this one is in an import cycle with, if any.
    /// A cycle has no internal reading order to offer, so the whole
    /// group is read together and the caller is told why.
    pub cycle_with: Vec<PathBuf>,
}

impl TourStep {
    /// A one-line, plain-English reason this file sits at this position.
    /// Built from the measured fields above — nothing here is inferred
    /// or generated.
    pub fn why(&self) -> String {
        let mut why = match self.role {
            StepRole::EntryPoint => {
                format!(
                    "entry point — nothing imports it; it pulls in {}",
                    plural(self.dependencies, "file")
                )
            }
            StepRole::Foundation => format!(
                "foundation — {} depend on it, it depends on nothing indexed",
                plural(self.dependents, "file")
            ),
            StepRole::Connector => format!(
                "connector — {} depend on it, it pulls in {}",
                plural(self.dependents, "file"),
                plural(self.dependencies, "file")
            ),
            StepRole::Isolated => {
                "isolated — no resolved imports either way (often unresolved, not standalone)"
                    .to_string()
            }
        };
        if !self.cycle_with.is_empty() {
            why.push_str(&format!(
                "; in an import cycle with {} — read the group together, it has no internal order",
                plural(self.cycle_with.len(), "other file")
            ));
        }
        if let Some(h) = self.health {
            if h < 5.0 {
                why.push_str(&format!("; health {h:.1}/10, budget extra time"));
            }
        }
        if let Some(hs) = self.hotspot {
            if hs > 0 {
                why.push_str(&format!("; hotspot score {hs}, it changes often"));
            }
        }
        why
    }
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// A complete tour: the ordered steps, plus enough context that a short
/// tour can't be mistaken for a complete picture of the repo.
#[derive(Debug, Clone)]
pub struct Tour {
    pub steps: Vec<TourStep>,
    /// How many files were eligible for selection. `steps.len()` is
    /// capped by [`TourOptions::max_steps`]; this is the honest
    /// denominator.
    pub considered: usize,
    /// Present when the tour was rooted at one file (`--from`): the file
    /// it was rooted at.
    pub rooted_at: Option<PathBuf>,
}

/// How to build a tour.
#[derive(Debug, Clone)]
pub struct TourOptions {
    /// Hard cap on steps. `0` means "no cap" — every eligible file, in
    /// dependency order.
    pub max_steps: usize,
    /// Restrict the tour to this file and everything it transitively
    /// imports: "what do I have to read to understand this one file".
    pub from: Option<PathBuf>,
}

impl Default for TourOptions {
    fn default() -> Self {
        Self {
            max_steps: DEFAULT_MAX_STEPS,
            from: None,
        }
    }
}

/// Everything measured about one candidate file, gathered once so the
/// ranking and the ordering read from the same numbers.
struct Candidate<'a> {
    record: &'a FileRecord,
    dependents: usize,
    dependencies: usize,
    health: Option<f64>,
    hotspot: Option<usize>,
}

impl Candidate<'_> {
    fn role(&self) -> StepRole {
        match (self.dependents, self.dependencies) {
            (0, 0) => StepRole::Isolated,
            (0, _) => StepRole::EntryPoint,
            (_, 0) => StepRole::Foundation,
            _ => StepRole::Connector,
        }
    }
}

/// Build a tour over `index`.
///
/// `health` and `hotspots` are optional enrichment: they change the
/// *ranking* (a load-bearing file that is also gnarly or churn-heavy
/// earns its place sooner) and populate [`TourStep::health`] /
/// [`TourStep::hotspot`], but a tour is fully computable without either.
/// That matters because hotspot data needs a git checkout, and this
/// port's rule elsewhere (`get_risk`, `/api/hotspots`) is to degrade
/// rather than error when the indexed root isn't a git repository.
pub fn build_tour(
    index: &RepoIndex,
    graph: &RepoGraph,
    health: Option<&HealthReport>,
    hotspots: &HashMap<PathBuf, usize>,
    opts: &TourOptions,
) -> anyhow::Result<Tour> {
    let health_by_file: HashMap<&Path, f64> = health
        .map(|r| {
            r.file_scores
                .iter()
                .map(|f| (f.file.as_path(), f.score))
                .collect()
        })
        .unwrap_or_default();

    let (rooted_at, scope) = match &opts.from {
        Some(from) => {
            let (start, closure) = dependency_closure(index, graph, from)?;
            (Some(start), Some(closure))
        }
        None => (None, None),
    };

    let mut candidates: Vec<Candidate> = index
        .files
        .iter()
        // A file with no extracted symbols is not something to send a
        // reader to: the Structural-tier languages (no grammar) and
        // empty files both land here. They still count in the index and
        // in git analytics, they just make pointless tour steps.
        .filter(|f| !f.symbols.is_empty())
        .filter(|f| scope.as_ref().is_none_or(|s| s.contains(&f.path)))
        .map(|record| Candidate {
            record,
            dependents: graph.dependents_of(&record.path).len(),
            dependencies: graph.dependencies_of(&record.path).len(),
            health: health_by_file.get(record.path.as_path()).copied(),
            hotspot: hotspots.get(&record.path).copied(),
        })
        .collect();

    let considered = candidates.len();
    rank_candidates(&mut candidates);
    select(&mut candidates, opts.max_steps);

    let steps = order_by_dependency(graph, candidates);

    Ok(Tour {
        steps,
        considered,
        // The index's own path form, not whatever the caller typed --
        // `--from lib.rs` and `--from ./src/lib.rs` must report
        // identically.
        rooted_at,
    })
}

/// At most this fraction of a tour's stops is reserved for entry points
/// (as `max_steps / ENTRY_POINT_SHARE`, always at least one). See
/// [`select`] for why the reservation exists at all.
const ENTRY_POINT_SHARE: usize = 5;

/// Cut the ranked candidates down to `max_steps` (`0` = keep everything).
///
/// A straight `truncate` off the ranking would produce a tour with **no
/// entry points in it at all**, which was worth catching: the ranking
/// leads with how many files import you, and an entry point by
/// definition has zero. Running this against this port's own workspace
/// gave 15 stops that were all `foundation`/`connector` — a tour of the
/// plumbing that never reaches anything you could actually run, and it
/// contradicted this module's own "closes on the entry points" claim.
///
/// So a slice of the tour ([`ENTRY_POINT_SHARE`]) is reserved for the
/// highest-ranked entry points, which displace the weakest non-entry
/// points that made the cut. The reservation is a ceiling, not a quota:
/// a repo with no entry points, or one whose entry points already ranked
/// in on their own, loses no slots to it.
fn select(candidates: &mut Vec<Candidate>, max_steps: usize) {
    if max_steps == 0 || candidates.len() <= max_steps {
        return;
    }
    let budget = (max_steps / ENTRY_POINT_SHARE).max(1);
    let already = candidates[..max_steps]
        .iter()
        .filter(|c| c.role() == StepRole::EntryPoint)
        .count();
    let wanted = budget.saturating_sub(already);

    if wanted > 0 {
        // Promote the best entry points sitting just outside the cut,
        // evicting the weakest non-entry-points inside it. Both ends are
        // taken in ranked order, so this stays deterministic.
        let promote: Vec<usize> = candidates
            .iter()
            .enumerate()
            .skip(max_steps)
            .filter(|(_, c)| c.role() == StepRole::EntryPoint)
            .map(|(i, _)| i)
            .take(wanted)
            .collect();
        let evict: Vec<usize> = candidates[..max_steps]
            .iter()
            .enumerate()
            .filter(|(_, c)| c.role() != StepRole::EntryPoint)
            .map(|(i, _)| i)
            .rev()
            .take(promote.len())
            .collect();
        for (&from, &to) in promote.iter().zip(evict.iter()) {
            candidates.swap(from, to);
        }
    }

    candidates.truncate(max_steps);
    // The swap above leaves the selection out of ranked order; the
    // ordering step reads ranks by position, so restore it.
    rank_candidates(candidates);
}

/// Rank candidates most-worth-reading first.
///
/// The primary signal is how many files import this one: that is the
/// measure of "you cannot understand this codebase without this file",
/// and it is the same signal `repowise overview` already leads with.
/// Health and hotspot act only as tie-breakers *within* an equal
/// dependent count — they say "this one is costly", not "this one is
/// structural", and a tour is ordered by structure first.
///
/// Every tie-break chain ends at the path, so the ranking is total and
/// the tour is byte-identical across runs and machines.
fn rank_candidates(candidates: &mut [Candidate]) {
    candidates.sort_by(|a, b| {
        // Isolated files last regardless of anything else: with no
        // resolved edges either way they cannot be placed meaningfully
        // in a dependency order.
        let a_isolated = a.role() == StepRole::Isolated;
        let b_isolated = b.role() == StepRole::Isolated;
        a_isolated
            .cmp(&b_isolated)
            .then(b.dependents.cmp(&a.dependents))
            .then(b.hotspot.unwrap_or(0).cmp(&a.hotspot.unwrap_or(0)))
            // Lower health score = worse = read sooner. `None` sorts as
            // 10.0 (the "no markers triggered" end) so an unmeasured
            // file never outranks a measured-unhealthy one on a signal
            // that was never computed for it.
            .then(
                a.health
                    .unwrap_or(10.0)
                    .total_cmp(&b.health.unwrap_or(10.0)),
            )
            .then(b.record.symbols.len().cmp(&a.record.symbols.len()))
            .then(a.record.path.cmp(&b.record.path))
    });
}

/// Order the selected candidates so that nothing appears before what it
/// is built out of.
///
/// Import cycles are collapsed into strongly-connected components first
/// — a cycle has no internal order to offer, and a plain topological
/// sort would simply fail on one. Each group is then emitted in
/// dependency order, with members of a collapsed group listed together
/// and flagged via [`TourStep::cycle_with`].
///
/// Among groups that are equally ready to be emitted, the one whose best
/// member ranked highest goes first — so the ordering constraint is
/// respected, and the ranking breaks every remaining tie.
fn order_by_dependency(graph: &RepoGraph, candidates: Vec<Candidate>) -> Vec<TourStep> {
    let rank_of: HashMap<&Path, usize> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (c.record.path.as_path(), i))
        .collect();
    // Induced subgraph: only the selected files, only edges between
    // them (`node_of` holds exactly the selection, so a lookup miss *is*
    // "not selected"). Edge direction matches the graph's own
    // (`A -> B` = A imports B).
    let mut sub: DiGraph<&Path, ()> = DiGraph::new();
    let mut node_of: HashMap<&Path, NodeIndex> = HashMap::new();
    for c in &candidates {
        let p = c.record.path.as_path();
        node_of.insert(p, sub.add_node(p));
    }
    for c in &candidates {
        let from = node_of[c.record.path.as_path()];
        for dep in graph.dependencies_of(&c.record.path) {
            if let Some(&to) = node_of.get(dep.as_path()) {
                if to != from {
                    sub.add_edge(from, to, ());
                }
            }
        }
    }

    // Collapse cycles. Every file lands in exactly one group; an
    // acyclic file is a group of one.
    let sccs = kosaraju_scc(&sub);
    let mut group_of: HashMap<&Path, usize> = HashMap::new();
    let mut groups: Vec<Vec<&Path>> = Vec::with_capacity(sccs.len());
    for scc in &sccs {
        let gid = groups.len();
        let mut members: Vec<&Path> = scc.iter().map(|&n| sub[n]).collect();
        // `kosaraju_scc` gives no order guarantee within a component;
        // impose our own so the tour stays deterministic.
        members.sort_by_key(|p| (rank_of[p], *p));
        for p in &members {
            group_of.insert(p, gid);
        }
        groups.push(members);
    }

    // Condensation: group -> groups it depends on. Kahn's algorithm over
    // the *reversed* condensation emits dependencies before dependents.
    let mut depends_on: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); groups.len()];
    let mut depended_by: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); groups.len()];
    for edge in sub.edge_indices() {
        let (a, b) = sub
            .edge_endpoints(edge)
            .expect("edge index from this graph");
        let (ga, gb) = (group_of[sub[a]], group_of[sub[b]]);
        if ga != gb {
            depends_on[ga].insert(gb);
            depended_by[gb].insert(ga);
        }
    }

    let best_rank: Vec<usize> = groups
        .iter()
        .map(|m| m.iter().map(|p| rank_of[p]).min().unwrap_or(usize::MAX))
        .collect();

    let mut remaining: Vec<usize> = depends_on.iter().map(|d| d.len()).collect();
    let mut ready: Vec<usize> = (0..groups.len()).filter(|&g| remaining[g] == 0).collect();
    let mut emitted: Vec<usize> = Vec::with_capacity(groups.len());
    let mut queue: VecDeque<usize> = VecDeque::new();

    while !ready.is_empty() || !queue.is_empty() {
        // Deterministic pick: among everything currently emittable, take
        // the group holding the highest-ranked file.
        ready.extend(queue.drain(..));
        ready.sort_by_key(|&g| (best_rank[g], g));
        let g = ready.remove(0);
        emitted.push(g);
        for &dependent in &depended_by[g] {
            remaining[dependent] -= 1;
            if remaining[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }

    let by_path: HashMap<&Path, &Candidate> = candidates
        .iter()
        .map(|c| (c.record.path.as_path(), c))
        .collect();

    let mut steps = Vec::with_capacity(candidates.len());
    for gid in emitted {
        let members = &groups[gid];
        for p in members {
            let c = by_path[p];
            steps.push(TourStep {
                position: steps.len() + 1,
                file: c.record.path.clone(),
                role: c.role(),
                dependents: c.dependents,
                dependencies: c.dependencies,
                symbols: c.record.symbols.len(),
                lines: c.record.lines,
                health: c.health,
                hotspot: c.hotspot,
                cycle_with: if members.len() > 1 {
                    members
                        .iter()
                        .filter(|other| *other != p)
                        .map(|o| o.to_path_buf())
                        .collect()
                } else {
                    Vec::new()
                },
            });
        }
    }
    steps
}

/// `from` plus everything it transitively imports (resolved edges only).
///
/// Errors rather than returning an empty tour when `from` isn't in the
/// index: "this file has no dependencies" and "you named a file I never
/// indexed" are different answers, and silently returning the first for
/// the second is the kind of false negative a reader would act on.
fn dependency_closure(
    index: &RepoIndex,
    graph: &RepoGraph,
    from: &Path,
) -> anyhow::Result<(PathBuf, HashSet<PathBuf>)> {
    let start = resolve_indexed_path(index, from)?;
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([start.clone()]);
    while let Some(file) = queue.pop_front() {
        if !seen.insert(file.clone()) {
            continue;
        }
        for dep in graph.dependencies_of(&file) {
            queue.push_back(dep);
        }
    }
    Ok((start, seen))
}

/// Match a caller-supplied path against the index, accepting either the
/// absolute form the index stores or a repo-relative one.
fn resolve_indexed_path(index: &RepoIndex, wanted: &Path) -> anyhow::Result<PathBuf> {
    let candidates = [wanted.to_path_buf(), index.root.join(wanted)];
    for c in &candidates {
        if let Some(f) = index.files.iter().find(|f| f.path == *c) {
            return Ok(f.path.clone());
        }
    }
    // Fall back to a suffix match so `--from lib.rs` works when it is
    // unambiguous, and says so plainly when it isn't.
    let matches: Vec<&FileRecord> = index
        .files
        .iter()
        .filter(|f| f.path.ends_with(wanted))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.path.clone()),
        [] => anyhow::bail!(
            "{} is not in the index -- run `repowise update` if it is new, or check the path",
            wanted.display()
        ),
        many => anyhow::bail!(
            "{} is ambiguous: {} indexed files end with it (e.g. {}) -- pass a longer path",
            wanted.display(),
            many.len(),
            many.iter()
                .take(3)
                .map(|f| f.path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
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

    /// `pkg/a.py` imports `pkg/b.py` imports `pkg/c.py`. `__init__.py` is
    /// empty, so it is indexed but never a tour stop.
    fn chain_fixture(root: &Path) {
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/__init__.py"), "").unwrap();
        fs::write(root.join("pkg/c.py"), "def c():\n    return 1\n").unwrap();
        fs::write(
            root.join("pkg/b.py"),
            "from pkg.c import c\n\ndef b():\n    return c()\n",
        )
        .unwrap();
        fs::write(
            root.join("pkg/a.py"),
            "from pkg.b import b\n\ndef a():\n    return b()\n",
        )
        .unwrap();
    }

    fn tour(root: &Path, opts: &TourOptions) -> Tour {
        let (index, graph) = build(root);
        build_tour(&index, &graph, None, &HashMap::new(), opts).unwrap()
    }

    fn names(tour: &Tour) -> Vec<String> {
        tour.steps
            .iter()
            .map(|s| s.file.file_name().unwrap().to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn dependencies_are_read_before_the_files_that_import_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        chain_fixture(&root);

        let t = tour(&root, &TourOptions::default());
        assert_eq!(names(&t), vec!["c.py", "b.py", "a.py"]);
        assert_eq!(t.steps[0].position, 1);
        assert_eq!(t.steps[2].position, 3);
    }

    #[test]
    fn roles_follow_the_resolved_edges() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        chain_fixture(&root);

        let t = tour(&root, &TourOptions::default());
        let roles: Vec<StepRole> = t.steps.iter().map(|s| s.role).collect();
        assert_eq!(
            roles,
            vec![
                StepRole::Foundation,
                StepRole::Connector,
                StepRole::EntryPoint
            ]
        );
        assert!(t.steps[0].why().contains("foundation"));
        assert!(t.steps[2].why().contains("entry point"));
    }

    /// A cycle has no internal reading order; the tour must still
    /// produce one total order rather than failing or dropping files.
    #[test]
    fn an_import_cycle_is_collapsed_and_flagged_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/__init__.py"), "").unwrap();
        fs::write(
            root.join("pkg/a.py"),
            "from pkg.b import b\n\ndef a():\n    return b()\n",
        )
        .unwrap();
        fs::write(
            root.join("pkg/b.py"),
            "from pkg.a import a\n\ndef b():\n    return a()\n",
        )
        .unwrap();

        let t = tour(&root, &TourOptions::default());
        assert_eq!(t.steps.len(), 2, "no file may be dropped by cycle collapse");
        for step in &t.steps {
            assert_eq!(
                step.cycle_with.len(),
                1,
                "{} should be flagged as cycling with the other",
                step.file.display()
            );
            assert!(step.why().contains("import cycle"), "{}", step.why());
        }
    }

    fn flat_fixture(root: &Path, n: usize) {
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/__init__.py"), "").unwrap();
        for i in 0..n {
            fs::write(
                root.join(format!("pkg/f{i}.py")),
                format!("def f{i}():\n    return {i}\n"),
            )
            .unwrap();
        }
    }

    #[test]
    fn max_steps_caps_the_tour_but_considered_reports_the_true_total() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        flat_fixture(&root, 10);

        let t = tour(
            &root,
            &TourOptions {
                max_steps: 3,
                from: None,
            },
        );
        assert_eq!(t.steps.len(), 3);
        assert_eq!(t.considered, 10, "the denominator must stay honest");
    }

    #[test]
    fn max_steps_zero_means_uncapped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        flat_fixture(&root, 10);

        let t = tour(
            &root,
            &TourOptions {
                max_steps: 0,
                from: None,
            },
        );
        assert_eq!(t.steps.len(), 10);
    }

    #[test]
    fn the_same_input_always_produces_the_same_tour() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        chain_fixture(&root);
        flat_fixture(&root, 8);

        let first = names(&tour(&root, &TourOptions::default()));
        for _ in 0..5 {
            assert_eq!(
                names(&tour(&root, &TourOptions::default())),
                first,
                "tour ordering must be stable across runs"
            );
        }
    }

    #[test]
    fn from_restricts_the_tour_to_one_files_dependency_closure() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        chain_fixture(&root);
        fs::write(root.join("pkg/unrelated.py"), "def u():\n    return 0\n").unwrap();

        let t = tour(
            &root,
            &TourOptions {
                max_steps: DEFAULT_MAX_STEPS,
                from: Some(PathBuf::from("pkg/b.py")),
            },
        );
        assert_eq!(
            names(&t),
            vec!["c.py", "b.py"],
            "the closure is b and what b imports -- not a, which imports b"
        );
        assert_eq!(t.rooted_at, Some(root.join("pkg/b.py")));
    }

    /// `--from b.py` should work when it is unambiguous, and report the
    /// index's own path form rather than what the caller typed.
    #[test]
    fn from_accepts_an_unambiguous_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        chain_fixture(&root);

        let t = tour(
            &root,
            &TourOptions {
                max_steps: DEFAULT_MAX_STEPS,
                from: Some(PathBuf::from("b.py")),
            },
        );
        assert_eq!(t.rooted_at, Some(root.join("pkg/b.py")));
    }

    #[test]
    fn from_an_unindexed_file_errors_rather_than_returning_an_empty_tour() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        chain_fixture(&root);

        let (index, graph) = build(&root);
        let err = build_tour(
            &index,
            &graph,
            None,
            &HashMap::new(),
            &TourOptions {
                max_steps: DEFAULT_MAX_STEPS,
                from: Some(PathBuf::from("ghost.py")),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("not in the index"), "{err}");
    }

    #[test]
    fn an_ambiguous_from_says_so_rather_than_picking_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("one")).unwrap();
        fs::create_dir_all(root.join("two")).unwrap();
        fs::write(root.join("one/dup.py"), "def a():\n    return 1\n").unwrap();
        fs::write(root.join("two/dup.py"), "def b():\n    return 2\n").unwrap();

        let (index, graph) = build(&root);
        let err = build_tour(
            &index,
            &graph,
            None,
            &HashMap::new(),
            &TourOptions {
                max_steps: DEFAULT_MAX_STEPS,
                from: Some(PathBuf::from("dup.py")),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("ambiguous"), "{err}");
    }

    /// Files with no extracted symbols (Structural-tier languages, empty
    /// files) are indexed but make pointless tour stops.
    #[test]
    fn files_without_symbols_are_not_tour_stops() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("real.py"), "def a():\n    return 1\n").unwrap();
        fs::write(root.join("empty.py"), "\n").unwrap();

        let t = tour(&root, &TourOptions::default());
        assert_eq!(names(&t), vec!["real.py"]);
        assert_eq!(t.considered, 1);
    }

    #[test]
    fn hotspot_breaks_ties_within_an_equal_dependent_count() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("cold.py"), "def cold():\n    return 1\n").unwrap();
        fs::write(root.join("hot.py"), "def hot():\n    return 1\n").unwrap();

        let (index, graph) = build(&root);
        let hotspots = HashMap::from([(root.join("hot.py"), 500usize)]);
        let t = build_tour(&index, &graph, None, &hotspots, &TourOptions::default()).unwrap();

        assert_eq!(
            names(&t),
            vec!["hot.py", "cold.py"],
            "with equal (zero) dependents, the hotspot ranks first"
        );
        assert_eq!(t.steps[0].hotspot, Some(500));
        assert_eq!(t.steps[1].hotspot, None, "unmeasured must stay None");
        assert!(t.steps[0].why().contains("changes often"));
    }

    /// `None` must read as "not measured", never as a healthy value that
    /// outranks a file we actually measured and found unhealthy.
    #[test]
    fn unmeasured_health_never_outranks_a_measured_unhealthy_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("unmeasured.py"), "def u():\n    return 1\n").unwrap();
        fs::write(root.join("bad.py"), "def b():\n    return 1\n").unwrap();

        let (index, graph) = build(&root);
        let report = HealthReport {
            file_scores: vec![repowise_health::FileHealth {
                file: root.join("bad.py"),
                score: 1.0,
                finding_count: 9,
            }],
            findings: Vec::new(),
            average_score: 1.0,
        };
        let t = build_tour(
            &index,
            &graph,
            Some(&report),
            &HashMap::new(),
            &TourOptions::default(),
        )
        .unwrap();

        assert_eq!(names(&t), vec!["bad.py", "unmeasured.py"]);
        assert_eq!(t.steps[0].health, Some(1.0));
        assert_eq!(t.steps[1].health, None, "unmeasured must stay None");
        assert!(t.steps[0].why().contains("budget extra time"));
    }

    /// Regression test for the bug this crate shipped with for exactly
    /// one manual run: ranked purely by dependent count, a tour of this
    /// port's own workspace selected 15 files that were all
    /// foundation/connector and never reached a single entry point.
    #[test]
    fn a_capped_tour_still_reaches_an_entry_point() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/__init__.py"), "").unwrap();
        // Ten files everyone imports, so they dominate the ranking...
        for i in 0..10 {
            fs::write(
                root.join(format!("pkg/base{i}.py")),
                format!("def base{i}():\n    return {i}\n"),
            )
            .unwrap();
        }
        // ...and one entry point that imports them all and is imported
        // by nothing, so a pure dependent-count ranking puts it last.
        let imports: String = (0..10)
            .map(|i| format!("from pkg.base{i} import base{i}\n"))
            .collect();
        fs::write(
            root.join("pkg/main.py"),
            format!("{imports}\ndef main():\n    return base0()\n"),
        )
        .unwrap();

        let t = tour(
            &root,
            &TourOptions {
                max_steps: 5,
                from: None,
            },
        );
        assert_eq!(t.steps.len(), 5);
        assert!(
            t.steps.iter().any(|s| s.role == StepRole::EntryPoint),
            "a capped tour must still reach somewhere runnable: {:?}",
            names(&t)
        );
        assert_eq!(
            t.steps.last().unwrap().role,
            StepRole::EntryPoint,
            "and the entry point closes the tour"
        );
    }

    /// The reservation is a ceiling, not a quota -- a repo with no entry
    /// points must not lose slots to it.
    #[test]
    fn the_entry_point_reservation_costs_nothing_when_there_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        flat_fixture(&root, 10);

        let t = tour(
            &root,
            &TourOptions {
                max_steps: 4,
                from: None,
            },
        );
        assert_eq!(t.steps.len(), 4);
        assert!(t.steps.iter().all(|s| s.role == StepRole::Isolated));
    }

    /// A file that many others import must be read before one nothing
    /// imports, even when the second is a raging hotspot -- structure
    /// outranks cost, and hotspot is only a tie-break.
    #[test]
    fn dependent_count_outranks_hotspot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        chain_fixture(&root);
        fs::write(root.join("pkg/loner.py"), "def l():\n    return 1\n").unwrap();

        let (index, graph) = build(&root);
        let hotspots = HashMap::from([(root.join("pkg/loner.py"), 9_999usize)]);
        let t = build_tour(
            &index,
            &graph,
            None,
            &hotspots,
            &TourOptions {
                max_steps: 2,
                from: None,
            },
        )
        .unwrap();

        assert!(
            !names(&t).contains(&"loner.py".to_string()),
            "an isolated file must not displace the depended-on chain: {:?}",
            names(&t)
        );
    }
}
