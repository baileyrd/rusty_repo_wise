//! Best-effort architectural-decision mining: extracts decisions from
//! two sources — `docs/adr/*.md` files and commit messages matching a
//! decision-like keyword heuristic — links each to the indexed
//! files/symbols its body mentions, and tracks supersession via an
//! ADR's `Status: ... Superseded by ADR-XXXX` line.
//!
//! Only 2 of the original repowise's 8 decision sources are implemented
//! here — a focused subset, not a shallow stub of all 8 (see the README
//! for which are deferred and why).

mod adr_files;
mod commits;
mod linking;

use repowise_core::RepoIndex;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionSource {
    Adr { file: PathBuf },
    CommitMessage { hash: String, author: String },
}

#[derive(Debug, Clone)]
pub struct DecisionRecord {
    pub id: String,
    pub title: String,
    pub source: DecisionSource,
    /// Raw `Status:` line value (ADR source only).
    pub status: Option<String>,
    /// Normalized `ADR-XXXX` this decision is superseded by, if its
    /// status line says so.
    pub superseded_by: Option<String>,
    /// Raw `Date:` line value (ADR source only).
    pub date: Option<String>,
    /// Full text used for linking to graph nodes (the whole ADR file,
    /// or the commit message/subject).
    pub body: String,
    pub linked_files: Vec<PathBuf>,
}

impl DecisionRecord {
    pub fn is_superseded(&self) -> bool {
        self.superseded_by.is_some()
    }
}

/// Mine decisions from `docs/adr/*.md` and decision-like commit messages
/// under `index.root`, linking each to the files/symbols its body
/// mentions. Missing `docs/adr/` or an unreadable git history degrade to
/// an empty result for that source rather than failing the whole call —
/// ADR mining and commit mining are independent sources.
pub fn mine(index: &RepoIndex) -> anyhow::Result<Vec<DecisionRecord>> {
    let mut records = adr_files::mine_adr_files(&index.root)?;

    let commits = repowise_git::collect_commits(&index.root).unwrap_or_default();
    records.extend(commits::mine_commit_decisions(&commits));

    for record in &mut records {
        record.linked_files = linking::link_to_index(&record.body, index);
    }

    Ok(records)
}
