//! Producer/consumer API contract matching -- the last of #64's five
//! bundled items, and a fully independent capability from the other
//! four: no cross-repo symbol resolution involved at all, just a
//! regex-based scan of each indexed file's raw text for a small, fixed
//! table of HTTP route-registration and HTTP-call patterns. Coarse and
//! heuristic by design, the same honesty this port already applies to
//! `unresolved_import_stems`/`repowise-adr`'s keyword-based commit
//! mining: a real implementation would need to parse each web
//! framework's actual route-registration semantics per language, which
//! this port has no such capability for (no route/HTTP-client AST
//! extraction exists anywhere else in this codebase). This is a
//! best-guess simplifying definition, not a claim of completeness --
//! false negatives (an unrecognized framework idiom) and false
//! positives (a route-shaped string that isn't actually a route) are
//! both expected.

use crate::ResolvedWorkspaceRepo;
use regex::Regex;
use repowise_core::RepoIndex;
use std::path::PathBuf;

/// One HTTP route a repo appears to register (a "producer"), inferred
/// from a recognized call shape in its source -- see the module doc
/// comment for the coarse/heuristic caveat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerRoute {
    pub repo: String,
    pub file: PathBuf,
    pub method: Option<String>,
    pub path: String,
}

/// One HTTP call a repo appears to make (a "consumer"), inferred from a
/// recognized call shape in its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerCall {
    pub repo: String,
    pub file: PathBuf,
    pub method: Option<String>,
    pub path: String,
}

/// A consumer call whose path matches a producer route registered in a
/// *different* repo -- a real cross-repo API contract, at least as far
/// as this heuristic can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractMatch {
    pub producer_repo: String,
    pub producer_file: PathBuf,
    pub consumer_repo: String,
    pub consumer_file: PathBuf,
    pub path: String,
}

/// Matched producer/consumer pairs and consumer calls with no known
/// producer anywhere in the workspace. An "unmatched" entry is not
/// necessarily a problem -- it may be a call to a genuinely external
/// API, or a producer this heuristic's pattern table simply doesn't
/// recognize -- so it's reported as a plain finding, not an error.
#[derive(Debug, Clone, Default)]
pub struct ContractsReport {
    pub matches: Vec<ContractMatch>,
    pub unmatched_consumers: Vec<ConsumerCall>,
}

struct ProducerPattern {
    regex: Regex,
    /// Capture group index for the path; the method group (if any) is
    /// always the group immediately before it.
    path_group: usize,
    method_group: Option<usize>,
}

fn producer_patterns() -> Vec<ProducerPattern> {
    vec![
        // axum: `.route("/path", get(handler))` -- this port's own
        // dashboard server registers routes exactly this way.
        ProducerPattern {
            regex: Regex::new(r#"\.route\(\s*"(/[^"]*)"\s*,\s*(get|post|put|delete|patch)"#)
                .unwrap(),
            path_group: 1,
            method_group: Some(2),
        },
        // Flask/FastAPI: `@app.get("/path")` / `@app.post("/path")`.
        ProducerPattern {
            regex: Regex::new(r#"@app\.(get|post|put|delete|patch)\(\s*['"](/[^'"]*)['"]"#)
                .unwrap(),
            path_group: 2,
            method_group: Some(1),
        },
        // Express: `app.get("/path", ...)` / `router.post("/path", ...)`.
        ProducerPattern {
            regex: Regex::new(
                r#"\b(?:app|router)\.(get|post|put|delete|patch)\(\s*['"](/[^'"]*)['"]"#,
            )
            .unwrap(),
            path_group: 2,
            method_group: Some(1),
        },
    ]
}

fn consumer_patterns() -> Vec<ProducerPattern> {
    vec![
        // JS `fetch("/path")` -- no reliable method in the call shape
        // itself (GET is the default, but an options-object second
        // argument could override it), so method is left unknown.
        ProducerPattern {
            regex: Regex::new(r#"\bfetch\(\s*['"]([^'"]*)['"]"#).unwrap(),
            path_group: 1,
            method_group: None,
        },
        // JS axios: `axios.get("/path")`.
        ProducerPattern {
            regex: Regex::new(r#"axios\.(get|post|put|delete|patch)\(\s*['"]([^'"]*)['"]"#)
                .unwrap(),
            path_group: 2,
            method_group: Some(1),
        },
        // Python `requests.get("/path")`.
        ProducerPattern {
            regex: Regex::new(r#"requests\.(get|post|put|delete|patch)\(\s*['"]([^'"]*)['"]"#)
                .unwrap(),
            path_group: 2,
            method_group: Some(1),
        },
        // Rust `ureq::get("/path")` -- this port's own repowise-git/
        // repowise-llm crates call external APIs exactly this way.
        ProducerPattern {
            regex: Regex::new(r#"ureq::(get|post|put|delete|patch)\(\s*"([^"]*)""#).unwrap(),
            path_group: 2,
            method_group: Some(1),
        },
    ]
}

/// The path portion of a URL or route string: strips a `scheme://host`
/// prefix if present, and any `?query`/`#fragment` suffix. Best-effort --
/// a malformed or unusual URL just passes through unchanged.
fn path_only(raw: &str) -> String {
    let without_query = raw.split(['?', '#']).next().unwrap_or(raw);
    let after_scheme = match without_query.find("://") {
        Some(idx) => &without_query[idx + 3..],
        None => without_query,
    };
    match after_scheme.find('/') {
        Some(idx) => after_scheme[idx..].to_string(),
        None => without_query.to_string(),
    }
}

/// Segment-wise path match: a producer segment starting with `:` or `{`
/// (a route template placeholder, e.g. `:id`/`{id}`) matches any
/// consumer segment; every other segment must match exactly. Different
/// segment counts never match.
fn paths_match(producer_path: &str, consumer_path: &str) -> bool {
    let producer_segments: Vec<&str> = producer_path.split('/').filter(|s| !s.is_empty()).collect();
    let consumer_segments: Vec<&str> = consumer_path.split('/').filter(|s| !s.is_empty()).collect();
    if producer_segments.len() != consumer_segments.len() {
        return false;
    }
    producer_segments
        .iter()
        .zip(consumer_segments.iter())
        .all(|(p, c)| p.starts_with(':') || p.starts_with('{') || p == c)
}

fn scan_file(
    repo: &str,
    file: &PathBuf,
    patterns: &[ProducerPattern],
) -> Vec<(Option<String>, String)> {
    let Ok(content) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for pattern in patterns {
        for caps in pattern.regex.captures_iter(&content) {
            let path = path_only(
                caps.get(pattern.path_group)
                    .map(|m| m.as_str())
                    .unwrap_or(""),
            );
            let method = pattern
                .method_group
                .and_then(|g| caps.get(g))
                .map(|m| m.as_str().to_lowercase());
            found.push((method, path));
        }
    }
    let _ = repo; // kept for signature symmetry / future per-repo filtering
    found
}

/// Everything one pass over the workspace found, including which repos
/// it couldn't read.
///
/// `unindexed_repos` is the field that matters and the reason this
/// struct exists. The scan skips any repo whose index won't load, which
/// silently removes both its routes and its calls from consideration --
/// so a workspace where half the repos were never indexed produces a
/// short contract list that looks exactly like a workspace whose
/// services genuinely don't talk to each other. Carrying the skipped
/// names out of the scan is what lets `workspace_diagnostics` tell
/// those two apart.
struct WorkspaceScan {
    producers: Vec<ProducerRoute>,
    consumers: Vec<ConsumerCall>,
    unindexed_repos: Vec<String>,
}

/// The one scan both `workspace_contracts` and `workspace_diagnostics`
/// run, so the two can never disagree about what was found.
fn scan_workspace(repos: &[ResolvedWorkspaceRepo]) -> WorkspaceScan {
    let producer_regexes = producer_patterns();
    let consumer_regexes = consumer_patterns();

    let mut producers: Vec<ProducerRoute> = Vec::new();
    let mut consumers: Vec<ConsumerCall> = Vec::new();
    let mut unindexed_repos: Vec<String> = Vec::new();

    for repo in repos {
        let Ok(index) = RepoIndex::load(&repo.path) else {
            unindexed_repos.push(repo.name.clone());
            continue;
        };
        for file in &index.files {
            for (method, path) in scan_file(&repo.name, &file.path, &producer_regexes) {
                producers.push(ProducerRoute {
                    repo: repo.name.clone(),
                    file: file.path.clone(),
                    method,
                    path,
                });
            }
            for (method, path) in scan_file(&repo.name, &file.path, &consumer_regexes) {
                consumers.push(ConsumerCall {
                    repo: repo.name.clone(),
                    file: file.path.clone(),
                    method,
                    path,
                });
            }
        }
    }

    WorkspaceScan {
        producers,
        consumers,
        unindexed_repos,
    }
}

/// Why one consumer call has no cross-repo contract.
///
/// The whole point of `workspace diagnostics`. `workspace-contracts`
/// reports an unmatched consumer as a single undifferentiated finding,
/// but these four cases mean completely different things and only two
/// of them are even problems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmatchedReason {
    /// A producer in *another* repo serves this path, but registers a
    /// different HTTP method. A real mismatch worth looking at.
    MethodMismatch,
    /// A producer serves this path, but only inside the consumer's own
    /// repo. **Not a gap**: an intra-repo call was never going to be a
    /// cross-repo contract. Reported separately precisely so it stops
    /// being counted as a missing one.
    SameRepoOnly,
    /// No producer anywhere in the workspace serves this path. Either a
    /// genuinely external API, or a producer idiom the pattern table
    /// doesn't recognize -- this scan cannot tell which, and says so
    /// rather than picking.
    NoProducerAnywhere,
}

impl UnmatchedReason {
    pub fn label(&self) -> &'static str {
        match self {
            UnmatchedReason::MethodMismatch => "method-mismatch",
            UnmatchedReason::SameRepoOnly => "same-repo-only",
            UnmatchedReason::NoProducerAnywhere => "no-producer-anywhere",
        }
    }

    /// What a reader should actually do about it.
    pub fn explanation(&self) -> &'static str {
        match self {
            UnmatchedReason::MethodMismatch => {
                "another repo serves this path but registers a different HTTP method"
            }
            UnmatchedReason::SameRepoOnly => {
                "served within the calling repo itself -- not a cross-repo contract, \
                 and not a gap"
            }
            UnmatchedReason::NoProducerAnywhere => {
                "no repo in this workspace registers this path -- either an external \
                 API, or a route idiom this scan's pattern table doesn't recognize"
            }
        }
    }
}

/// One consumer call that produced no contract, and why.
#[derive(Debug, Clone)]
pub struct UnmatchedConsumer {
    pub call: ConsumerCall,
    pub reason: UnmatchedReason,
}

/// A route registered by some repo that nothing in the workspace calls.
///
/// Ambiguous on purpose: it's either dead surface, or a consumer this
/// scan's pattern table missed. Reported so a reader can decide, not
/// labelled as dead code.
#[derive(Debug, Clone)]
pub struct OrphanProducer {
    pub route: ProducerRoute,
}

/// Per-repo counts of what the scan actually found.
#[derive(Debug, Clone)]
pub struct RepoEndpointCounts {
    pub repo: String,
    pub producers: usize,
    pub consumers: usize,
    /// `false` when this repo's index couldn't be loaded -- its zeros
    /// above mean "not looked at", not "nothing there".
    pub indexed: bool,
}

/// Why the cross-repo contract link count is what it is.
///
/// Answers the question `workspace-contracts` leaves open: a short or
/// empty match list is either an architecture finding or a tooling
/// artifact, and until now those looked identical.
#[derive(Debug, Clone, Default)]
pub struct ContractDiagnostics {
    pub repos: Vec<RepoEndpointCounts>,
    pub matches: usize,
    pub unmatched_consumers: Vec<UnmatchedConsumer>,
    pub orphan_producers: Vec<OrphanProducer>,
}

impl ContractDiagnostics {
    /// Unmatched-consumer counts per reason, highest first.
    pub fn unmatched_by_reason(&self) -> Vec<(UnmatchedReason, usize)> {
        let reasons = [
            UnmatchedReason::NoProducerAnywhere,
            UnmatchedReason::SameRepoOnly,
            UnmatchedReason::MethodMismatch,
        ];
        let mut out: Vec<(UnmatchedReason, usize)> = reasons
            .into_iter()
            .map(|r| {
                (
                    r,
                    self.unmatched_consumers
                        .iter()
                        .filter(|u| u.reason == r)
                        .count(),
                )
            })
            .filter(|(_, n)| *n > 0)
            .collect();
        out.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        out
    }

    /// Repos the scan couldn't read at all.
    ///
    /// Checked first when reading a thin report: every one of these
    /// contributed zero routes and zero calls, so the contract count is
    /// a floor, not a finding.
    pub fn unindexed_repos(&self) -> Vec<&str> {
        self.repos
            .iter()
            .filter(|r| !r.indexed)
            .map(|r| r.repo.as_str())
            .collect()
    }
}

/// Explain the cross-repo contract link count: per-repo producer and
/// consumer counts, every unmatched consumer classified by *why* it
/// didn't match, and producers nothing calls.
///
/// Shares [`scan_workspace`] with [`workspace_contracts`], so the two
/// cannot disagree about what was found.
pub fn workspace_diagnostics(repos: &[ResolvedWorkspaceRepo]) -> ContractDiagnostics {
    let scan = scan_workspace(repos);

    let counts = repos
        .iter()
        .map(|repo| RepoEndpointCounts {
            repo: repo.name.clone(),
            producers: scan
                .producers
                .iter()
                .filter(|p| p.repo == repo.name)
                .count(),
            consumers: scan
                .consumers
                .iter()
                .filter(|c| c.repo == repo.name)
                .count(),
            indexed: !scan.unindexed_repos.contains(&repo.name),
        })
        .collect();

    let mut matches = 0usize;
    let mut unmatched_consumers = Vec::new();
    // Producer indices that some consumer matched, so orphans are
    // "nothing matched this", not "nothing matched this path".
    let mut matched_producers = vec![false; scan.producers.len()];

    for consumer in &scan.consumers {
        let path_hit = |p: &ProducerRoute| paths_match(&p.path, &consumer.path);
        let method_hit = |p: &ProducerRoute| match (&p.method, &consumer.method) {
            (Some(pm), Some(cm)) => pm == cm,
            _ => true,
        };

        if let Some(i) = scan
            .producers
            .iter()
            .position(|p| p.repo != consumer.repo && path_hit(p) && method_hit(p))
        {
            matches += 1;
            matched_producers[i] = true;
            continue;
        }

        // Ordered most-specific first: a cross-repo producer that only
        // differs by method is a sharper finding than "nothing serves
        // this", and an intra-repo hit isn't a finding at all.
        let reason = if scan
            .producers
            .iter()
            .any(|p| p.repo != consumer.repo && path_hit(p))
        {
            UnmatchedReason::MethodMismatch
        } else if scan
            .producers
            .iter()
            .any(|p| p.repo == consumer.repo && path_hit(p))
        {
            UnmatchedReason::SameRepoOnly
        } else {
            UnmatchedReason::NoProducerAnywhere
        };

        unmatched_consumers.push(UnmatchedConsumer {
            call: consumer.clone(),
            reason,
        });
    }

    let orphan_producers = scan
        .producers
        .into_iter()
        .zip(matched_producers)
        .filter(|(_, matched)| !matched)
        .map(|(route, _)| OrphanProducer { route })
        .collect();

    ContractDiagnostics {
        repos: counts,
        matches,
        unmatched_consumers,
        orphan_producers,
    }
}

/// Regex-scans every workspace repo's already-indexed files for
/// recognized HTTP producer/consumer call shapes, then matches each
/// consumer call against every OTHER repo's producer routes. See the
/// module doc comment for why this is coarse and heuristic by design.
pub fn workspace_contracts(repos: &[ResolvedWorkspaceRepo]) -> ContractsReport {
    let scan = scan_workspace(repos);
    let (producers, consumers) = (scan.producers, scan.consumers);

    let mut matches = Vec::new();
    let mut unmatched_consumers = Vec::new();
    for consumer in consumers {
        let found = producers.iter().find(|p| {
            p.repo != consumer.repo
                && paths_match(&p.path, &consumer.path)
                && match (&p.method, &consumer.method) {
                    (Some(pm), Some(cm)) => pm == cm,
                    _ => true,
                }
        });
        match found {
            Some(producer) => matches.push(ContractMatch {
                producer_repo: producer.repo.clone(),
                producer_file: producer.file.clone(),
                consumer_repo: consumer.repo.clone(),
                consumer_file: consumer.file.clone(),
                path: consumer.path.clone(),
            }),
            None => unmatched_consumers.push(consumer),
        }
    }

    ContractsReport {
        matches,
        unmatched_consumers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_only_strips_scheme_host_and_query() {
        assert_eq!(path_only("https://example.com/api/foo?x=1"), "/api/foo");
        assert_eq!(path_only("/api/foo"), "/api/foo");
        assert_eq!(path_only("/api/foo#section"), "/api/foo");
    }

    #[test]
    fn paths_match_treats_colon_and_brace_segments_as_wildcards() {
        assert!(paths_match("/api/users/:id", "/api/users/42"));
        assert!(paths_match("/api/users/{id}", "/api/users/42"));
        assert!(!paths_match("/api/users/:id", "/api/users"));
        assert!(!paths_match("/api/users", "/api/orders"));
    }
}
