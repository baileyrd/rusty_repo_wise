//! The `cli` decision source (issue #66, split from an also-rejected
//! `session` transcript-mining source, which needed a coding-agent
//! transcript format this port has never had anything to produce): a
//! decision the user typed in directly via `repowise decide`, not mined
//! from any artifact and not inferred by a model.
//!
//! The most trustworthy source of the eight, and the plainest to
//! implement: no anchor-checking ([`crate::inferred`]'s whole reason to
//! exist), because there's no model in the loop to verify against
//! reality -- this is the user's own stated intent, taken at face value
//! the same way a hand-written ADR file already is.
//!
//! # Append-only, unlike `inferred`
//!
//! `repowise-llm`'s inferred-decision pass replaces its store wholesale
//! on every run, because it's re-deriving a snapshot judgment each time.
//! A manually recorded decision is the opposite: each `repowise decide`
//! call is a single, deliberate, permanent record, and a later call must
//! never erase an earlier one.

use crate::{DecisionRecord, DecisionSource};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where `repowise decide` appends records, relative to the index dir.
pub const MANUAL_FILE: &str = "manual-decisions.json";

pub fn store_path(root: &Path) -> PathBuf {
    root.join(repowise_core::RepoIndex::INDEX_DIR)
        .join(MANUAL_FILE)
}

/// One decision the user typed in directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualDecision {
    /// `MANUAL-0001`-style sequential id, assigned at record time --
    /// mirroring ADR's own `ADR-XXXX` numbering, needed because a
    /// manually-typed title/rationale pair has no other guaranteed-unique
    /// key.
    pub id: String,
    pub title: String,
    pub rationale: String,
    /// RFC 3339 timestamp, passed in by the caller (the CLI) rather than
    /// computed here -- keeps this module a pure function of its
    /// arguments.
    pub recorded_at: String,
}

/// The decisions on disk. Append-only: see the module doc for why a
/// later `repowise decide` call must never erase an earlier record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManualDecisionStore {
    pub decisions: Vec<ManualDecision>,
}

impl ManualDecisionStore {
    /// A missing or unreadable store is an empty one, not an error --
    /// `repowise decide` hasn't been run yet is the default state.
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(store_path(root))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> anyhow::Result<PathBuf> {
        let path = store_path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(path)
    }

    /// Append one decision and persist, assigning the next sequential id.
    pub fn record(
        &mut self,
        root: &Path,
        title: String,
        rationale: String,
        recorded_at: String,
    ) -> anyhow::Result<ManualDecision> {
        let id = format!("MANUAL-{:04}", self.decisions.len() + 1);
        let decision = ManualDecision {
            id,
            title,
            rationale,
            recorded_at,
        };
        self.decisions.push(decision.clone());
        self.save(root)?;
        Ok(decision)
    }
}

/// Read the store and turn every record into a `DecisionRecord`. Unlike
/// `inferred`, nothing here is dropped -- there's no anchor to go stale,
/// since a manually recorded decision isn't a claim about a specific
/// line of code that could be rewritten out from under it.
pub fn mine_manual_decisions(root: &Path) -> Vec<DecisionRecord> {
    ManualDecisionStore::load(root)
        .decisions
        .into_iter()
        .map(|d| {
            DecisionRecord::new(
                d.id,
                d.title,
                DecisionSource::Manual {
                    recorded_at: d.recorded_at,
                },
                d.rationale,
            )
            // linked_files stays empty here -- filled in by
            // `mine_reporting`'s text-linking pass, since a manually
            // typed decision has no inherently known file location the
            // way a code comment or PR does.
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_store_is_the_default_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let store = ManualDecisionStore::load(&root);
        assert!(store.decisions.is_empty());
        assert!(mine_manual_decisions(&root).is_empty());
    }

    #[test]
    fn record_assigns_sequential_ids_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let mut store = ManualDecisionStore::load(&root);
        let first = store
            .record(
                &root,
                "Use SQLite for local state".to_string(),
                "No server to run; a file is enough for one machine.".to_string(),
                "2026-01-01T00:00:00+00:00".to_string(),
            )
            .unwrap();
        assert_eq!(first.id, "MANUAL-0001");

        let second = store
            .record(
                &root,
                "Vendor the schema migrations".to_string(),
                "Keeps upgrades reproducible without a network fetch.".to_string(),
                "2026-01-02T00:00:00+00:00".to_string(),
            )
            .unwrap();
        assert_eq!(second.id, "MANUAL-0002");

        // Persisted, not just held in memory.
        let reloaded = ManualDecisionStore::load(&root);
        assert_eq!(reloaded.decisions.len(), 2);
        assert_eq!(reloaded.decisions[0].id, "MANUAL-0001");
        assert_eq!(reloaded.decisions[1].id, "MANUAL-0002");
    }

    #[test]
    fn recording_never_erases_an_earlier_record() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let mut store = ManualDecisionStore::load(&root);
        store
            .record(
                &root,
                "First".to_string(),
                "First rationale.".to_string(),
                "2026-01-01T00:00:00+00:00".to_string(),
            )
            .unwrap();

        // A fresh load + record, as a second `repowise decide` process
        // invocation would do -- must append, not replace.
        let mut store = ManualDecisionStore::load(&root);
        store
            .record(
                &root,
                "Second".to_string(),
                "Second rationale.".to_string(),
                "2026-01-02T00:00:00+00:00".to_string(),
            )
            .unwrap();

        let reloaded = ManualDecisionStore::load(&root);
        assert_eq!(reloaded.decisions.len(), 2);
        assert_eq!(reloaded.decisions[0].title, "First");
        assert_eq!(reloaded.decisions[1].title, "Second");
    }

    #[test]
    fn mine_manual_decisions_reports_every_record_unfiltered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let mut store = ManualDecisionStore::load(&root);
        store
            .record(
                &root,
                "Use SQLite for local state".to_string(),
                "No server to run; a file is enough for one machine.".to_string(),
                "2026-01-01T00:00:00+00:00".to_string(),
            )
            .unwrap();

        let records = mine_manual_decisions(&root);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "MANUAL-0001");
        assert_eq!(records[0].title, "Use SQLite for local state");
        assert_eq!(
            records[0].body,
            "No server to run; a file is enough for one machine."
        );
        assert!(records[0].linked_files.is_empty());
        let DecisionSource::Manual { recorded_at } = &records[0].source else {
            panic!("wrong source: {:?}", records[0].source);
        };
        assert_eq!(recorded_at, "2026-01-01T00:00:00+00:00");
    }
}
