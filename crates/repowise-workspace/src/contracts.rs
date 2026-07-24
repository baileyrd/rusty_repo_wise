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

/// Regex-scans every workspace repo's already-indexed files for
/// recognized HTTP producer/consumer call shapes, then matches each
/// consumer call against every OTHER repo's producer routes. See the
/// module doc comment for why this is coarse and heuristic by design.
pub fn workspace_contracts(repos: &[ResolvedWorkspaceRepo]) -> ContractsReport {
    let producer_regexes = producer_patterns();
    let consumer_regexes = consumer_patterns();

    let mut producers: Vec<ProducerRoute> = Vec::new();
    let mut consumers: Vec<ConsumerCall> = Vec::new();

    for repo in repos {
        let Ok(index) = RepoIndex::load(&repo.path) else {
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
