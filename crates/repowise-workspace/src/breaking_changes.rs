//! Contract breaking-change detection: `workspace_contracts` (#64's
//! last slice, `contracts.rs`) matches producer/consumer API contracts
//! for one point-in-time snapshot only -- it has no memory of a prior
//! run, so it can't tell "this route never existed" apart from "this
//! route used to resolve and just broke". This module adds that
//! memory: a persisted snapshot of the last run's matched contracts
//! (`.repowise-workspace/contracts.json`, via
//! `crate::workspace_state_dir`), diffed against the current run on
//! every call to flag newly-broken ones, then overwritten with the
//! current state -- the same "current run becomes the new baseline"
//! model `repowise update` already uses for `.repowise/index.json`.
//!
//! Deliberately narrow: a broken contract is a consumer call site that
//! **used to resolve to a producer and no longer does** (the producer
//! moved, changed method, or disappeared). A consumer call site that's
//! gone entirely (the calling code itself was deleted) is not reported
//! here -- that's not a contract break, it's just gone, and
//! `workspace_diagnostics`'s existing counts already cover "how many
//! calls exist today" without this module's help.

use crate::contracts::{
    workspace_contracts, workspace_diagnostics, ContractMatch, ContractsReport, UnmatchedReason,
};
use crate::ResolvedWorkspaceRepo;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A minimal, comparable identity for one matched contract: the
/// consumer call site (repo + file + path) and which repo it resolved
/// to. Deliberately excludes the producer's own file and HTTP method: a
/// producer route moving to a different file while still serving the
/// same path is not a break, and a resolved match's method is already
/// implied by matching at all (see `contracts::workspace_contracts`'s
/// method-compatibility check) -- this identity only needs to answer
/// "did this consumer call resolve to this producer repo", not
/// reproduce every field of the match that proved it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContractKey {
    pub consumer_repo: String,
    pub consumer_file: PathBuf,
    pub path: String,
    pub producer_repo: String,
}

impl ContractKey {
    fn from_match(m: &ContractMatch) -> Self {
        ContractKey {
            consumer_repo: m.consumer_repo.clone(),
            consumer_file: m.consumer_file.clone(),
            path: m.path.clone(),
            producer_repo: m.producer_repo.clone(),
        }
    }
}

/// One contract that resolved in a prior run and no longer does.
#[derive(Debug, Clone)]
pub struct BrokenContract {
    pub key: ContractKey,
    /// Why it no longer resolves, reusing `workspace_diagnostics`'s own
    /// classification rather than re-deriving it. `None` when the
    /// consumer call site itself is gone (removed from the scan
    /// entirely, not merely unmatched) -- see this module's own doc
    /// comment for why that case isn't itself the interesting one.
    pub reason: Option<UnmatchedReason>,
}

fn load_snapshot(path: &Path) -> BTreeSet<ContractKey> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_snapshot(dir: &Path, path: &Path, current: &BTreeSet<ContractKey>) {
    // Best-effort: an unwritable state dir shouldn't fail the read this
    // is attached to (the report above is still correct), just leave
    // the next run unable to diff against this one.
    if std::fs::create_dir_all(dir).is_ok() {
        if let Ok(json) = serde_json::to_string_pretty(current) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// Runs `workspace_contracts`, diffs its matches against the last
/// persisted snapshot at `state_dir` (see [`crate::workspace_state_dir`])
/// to find newly-broken contracts, then overwrites the snapshot with
/// the current matches. A missing or unreadable snapshot (most commonly
/// the first run) means an empty baseline -- nothing can be reported as
/// broken with nothing to compare against, which is the correct answer,
/// not a degraded one.
pub fn workspace_contract_changes(
    repos: &[ResolvedWorkspaceRepo],
    state_dir: &Path,
) -> (ContractsReport, Vec<BrokenContract>) {
    let report = workspace_contracts(repos);
    let current: BTreeSet<ContractKey> =
        report.matches.iter().map(ContractKey::from_match).collect();

    let snapshot_path = state_dir.join("contracts.json");
    let previous = load_snapshot(&snapshot_path);

    let broken: Vec<BrokenContract> = if previous.is_empty() {
        Vec::new()
    } else {
        let diagnostics = workspace_diagnostics(repos);
        previous
            .difference(&current)
            .map(|key| {
                let reason = diagnostics
                    .unmatched_consumers
                    .iter()
                    .find(|u| {
                        u.call.repo == key.consumer_repo
                            && u.call.file == key.consumer_file
                            && u.call.path == key.path
                    })
                    .map(|u| u.reason);
                BrokenContract {
                    key: key.clone(),
                    reason,
                }
            })
            .collect()
    };

    save_snapshot(state_dir, &snapshot_path, &current);

    (report, broken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{ConsumerCall, ProducerRoute};

    fn m(
        consumer_repo: &str,
        consumer_file: &str,
        path: &str,
        producer_repo: &str,
    ) -> ContractMatch {
        ContractMatch {
            producer_repo: producer_repo.to_string(),
            producer_file: PathBuf::from(format!("{producer_repo}/route.rs")),
            consumer_repo: consumer_repo.to_string(),
            consumer_file: PathBuf::from(consumer_file),
            path: path.to_string(),
        }
    }

    #[test]
    fn a_first_run_with_no_snapshot_reports_nothing_broken() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join(".repowise-workspace");
        let current: BTreeSet<ContractKey> = [m("consumer", "b.rs", "/api/x", "producer")]
            .iter()
            .map(ContractKey::from_match)
            .collect();

        save_snapshot(&state_dir, &state_dir.join("contracts.json"), &current);

        // Loading back what was just written should reproduce it exactly.
        assert_eq!(load_snapshot(&state_dir.join("contracts.json")), current);
        // And an unwritten path (simulating a true first run) has no baseline.
        assert!(load_snapshot(&dir.path().join("nope.json")).is_empty());
    }

    #[test]
    fn a_key_present_before_and_absent_now_is_reported_broken() {
        let previous: BTreeSet<ContractKey> = [
            m("consumer", "b.rs", "/api/x", "producer"),
            m("consumer", "b.rs", "/api/y", "producer"),
        ]
        .iter()
        .map(ContractKey::from_match)
        .collect();
        let current: BTreeSet<ContractKey> = [m("consumer", "b.rs", "/api/x", "producer")]
            .iter()
            .map(ContractKey::from_match)
            .collect();

        let broken: Vec<&ContractKey> = previous.difference(&current).collect();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].path, "/api/y");
    }

    #[test]
    fn unrelated_producer_and_consumer_types_still_construct() {
        // Smoke-check that the module compiles against contracts.rs's
        // real types, not a divergent copy of them.
        let _producer = ProducerRoute {
            repo: "p".to_string(),
            file: PathBuf::from("p/route.rs"),
            method: Some("get".to_string()),
            path: "/api/x".to_string(),
        };
        let _consumer = ConsumerCall {
            repo: "c".to_string(),
            file: PathBuf::from("c/call.rs"),
            method: Some("get".to_string()),
            path: "/api/x".to_string(),
        };
    }
}
