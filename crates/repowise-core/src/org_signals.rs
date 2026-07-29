//! Per-file organizational-history data, computed by `repowise-git` and
//! injected into `repowise-health`'s fixed-penalty Organizational-signal
//! markers (issue #313, split from #62 -- see that issue for why these
//! don't need the ML calibration #62 rejected: every field here is a
//! plain measured count, not a trained weight).
//!
//! This lives in `repowise-core` rather than `repowise-git` for the same
//! reason [`crate::coverage::CoverageData`] does: `repowise-health`
//! already depends on `repowise-core` for [`crate::RepoIndex`], and
//! keeping this data type here means `repowise-health` never gains a
//! `repowise-git` dependency. The same "caller computes the external
//! signal, `repowise-health` only scores it" split that
//! `analyze_with_context`'s `hot_files`/`coverage` parameters already
//! established for issues #186/#243.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Everything `repowise-health`'s organizational-signal markers need,
/// one file's worth per map entry. A file absent from a map has no
/// signal there (0 churn, 0 bug-fix commits, etc.) -- consistent with
/// `repowise_git::Hotspot`'s existing "0 = no git history" convention,
/// not a special missing-data case.
#[derive(Debug, Clone, Default)]
pub struct OrgSignals {
    /// Commit count touching each file (`GitAnalytics::churn_of`).
    pub churn: BTreeMap<PathBuf, usize>,
    /// Bug-fix-flagged commit count touching each file
    /// (`GitAnalytics::bugfix_commits_of`) -- the `prior_defect` signal
    /// directly.
    pub bugfix_commits: BTreeMap<PathBuf, usize>,
    /// Authors needed to reach a majority of each file's blamed lines
    /// (`repowise_git::bus_factor`). `0` for a file with no blameable
    /// history, distinct from a real bus factor of `1`.
    pub bus_factor: BTreeMap<PathBuf, usize>,
    /// Count of distinct files each one co-changed with at least
    /// `repowise_git::org_signals::MIN_CO_CHANGE` times -- the
    /// `co_change_scatter` signal.
    pub co_change_partner_count: BTreeMap<PathBuf, usize>,
    /// Every file pair that co-changed at least
    /// `repowise_git::org_signals::MIN_CO_CHANGE` times, for
    /// `hidden_coupling` to cross-reference against the dependency
    /// graph -- which `repowise-health` already has via its own `graph`
    /// parameter, so this crate doesn't need to resolve imports itself.
    pub co_changed_pairs: Vec<(PathBuf, PathBuf, usize)>,
    /// Distinct authors touching each file within
    /// `repowise_git::org_signals::RECENT_WINDOW_DAYS` of collection
    /// time -- the `developer_congestion` signal.
    pub recent_author_count: BTreeMap<PathBuf, usize>,
}

impl OrgSignals {
    pub fn churn_of(&self, file: &std::path::Path) -> usize {
        self.churn.get(file).copied().unwrap_or(0)
    }

    pub fn bugfix_commits_of(&self, file: &std::path::Path) -> usize {
        self.bugfix_commits.get(file).copied().unwrap_or(0)
    }

    pub fn bus_factor_of(&self, file: &std::path::Path) -> usize {
        self.bus_factor.get(file).copied().unwrap_or(0)
    }

    pub fn co_change_partner_count_of(&self, file: &std::path::Path) -> usize {
        self.co_change_partner_count.get(file).copied().unwrap_or(0)
    }

    pub fn recent_author_count_of(&self, file: &std::path::Path) -> usize {
        self.recent_author_count.get(file).copied().unwrap_or(0)
    }
}
