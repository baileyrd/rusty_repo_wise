//! The `_meta` block attached to every tool response.
//!
//! # Why this exists
//!
//! Every tool in this server answers from `.repowise/index.json` —
//! whatever happens to be on disk. Without this block, an answer built
//! from an index stamped three months and four hundred commits ago is
//! indistinguishable, at the protocol level, from one built against
//! HEAD. That is the single way this server can actively mislead a
//! caller: not by erroring, but by answering confidently and staleley.
//!
//! `repowise status` already computes this signal for humans. This
//! carries the same signal to agents.
//!
//! # The quiet-by-default rule
//!
//! [`Meta`] skips [`Meta::live_head`] and [`Meta::stale_warning`] when
//! there is nothing to say, matching the reference's shape. That is a
//! deliberate design choice, not an optimization: a `stale_warning:
//! null` on every fresh response teaches a caller to stop reading the
//! field, so by the time a real warning appears it is one more key in
//! the noise. Absence is the "fine" signal; presence always means
//! something.
//!
//! # Unknown is not "fine"
//!
//! [`Meta::indexed_commit`] is `None` for an index built without git,
//! or one written before the field existed. That is reported as unknown
//! and deliberately produces **no** staleness claim in either
//! direction — neither a warning (which would fire on every non-git
//! directory) nor silence implying freshness (which would let every
//! pre-existing index claim to be current forever).

use repowise_core::RepoIndex;
use serde::Serialize;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Age past which an index is called out even with no commit to compare
/// against.
///
/// ~90 days. Only used on the no-git path: when [`Meta::indexed_commit`]
/// and the live HEAD are both known, the SHA comparison is exact and
/// this threshold is irrelevant — a one-day-old index that HEAD has
/// moved past is stale, and a year-old index on an untouched repo is
/// not. This is the fallback for when there is no commit to compare, and
/// it is a coarse heuristic on purpose rather than a tuned number.
const STALE_AGE_DAYS: u64 = 90;

const SECONDS_PER_DAY: u64 = 60 * 60 * 24;

/// Provenance and freshness for one tool response.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema, Default, PartialEq)]
pub struct Meta {
    /// Wall-time for this tool call, in milliseconds.
    pub timing_ms: u64,
    /// Whole days since the index file was last written. `None` when the
    /// index file's mtime can't be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_age_days: Option<u64>,
    /// The commit the index was built against (12-char prefix). `None`
    /// means unknown — no git, no commits, or an index predating the
    /// field. It does **not** mean "matches".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_commit: Option<String>,
    /// The repo's current HEAD — present **only when it differs** from
    /// `indexed_commit`. Its absence is the common, healthy case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_head: Option<String>,
    /// Set only on a real signal: a HEAD mismatch, or an index older
    /// than [`STALE_AGE_DAYS`] with no commit to compare against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_warning: Option<String>,
    /// Present only when `true`: this response reused the in-memory
    /// index/graph rather than re-reading and re-resolving from disk.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cached: bool,
}

/// Days since `path` was last modified, or `None` if that can't be read.
///
/// Truncating division, so an index written this morning reports 0 days
/// rather than rounding up to 1 and implying it is older than it is.
fn age_days(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let elapsed = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    Some(elapsed.as_secs() / SECONDS_PER_DAY)
}

impl Meta {
    /// The block for a tool whose answer does **not** come from this
    /// server's index — `get_change_risk` (git only) and the
    /// workspace tools (other repos' indexes).
    ///
    /// Reports timing and nothing else, on purpose. Attaching this
    /// index's freshness to an answer that didn't consult it would be
    /// misleading in the opposite direction from the problem `_meta`
    /// solves: a caller would discount a perfectly current git answer
    /// because an unrelated index happened to be old.
    pub fn timing_only(timing_ms: u64) -> Self {
        Self {
            timing_ms,
            ..Self::default()
        }
    }

    /// Build the block for one call.
    ///
    /// `root` is the indexed repo root; `cached` says whether this call
    /// avoided a re-read. Every field degrades to absent rather than to
    /// a guess — this runs on every tool call, so it must never be the
    /// thing that fails one.
    pub fn build(root: &Path, index: &RepoIndex, timing_ms: u64, cached: bool) -> Self {
        let index_age_days = age_days(&RepoIndex::index_path(root));
        let indexed_commit = index.indexed_commit.clone();
        let live = repowise_git::head_sha(root);

        // Only a *known* pair that disagrees is a mismatch. If either
        // side is unknown there is nothing to compare, and saying
        // nothing is the honest answer.
        let mismatched = match (&indexed_commit, &live) {
            (Some(indexed), Some(head)) => indexed != head,
            _ => false,
        };

        let live_head = if mismatched { live.clone() } else { None };

        let stale_warning = if mismatched {
            Some(format!(
                "Index was built against {}, but HEAD is now {}. \
                 Results describe the indexed commit, not the working tree. \
                 Run `repowise update` to re-index.",
                indexed_commit.as_deref().unwrap_or("an unknown commit"),
                live.as_deref().unwrap_or("unknown"),
            ))
        } else if indexed_commit.is_none() && index_age_days.is_some_and(|d| d >= STALE_AGE_DAYS) {
            // No commit to compare against, so age is all there is to go
            // on. Say which check actually ran, so nobody reads this as
            // a verified mismatch.
            Some(format!(
                "Index is {} days old and records no commit to compare against, \
                 so its freshness could not be verified. Run `repowise update` to re-index.",
                index_age_days.unwrap_or_default(),
            ))
        } else {
            None
        };

        Self {
            timing_ms,
            index_age_days,
            indexed_commit,
            live_head,
            stale_warning,
            cached,
        }
    }
}

/// A tool payload plus its `_meta` block.
///
/// `#[serde(flatten)]` keeps the payload's own fields at the top level,
/// so adding this to a tool is additive for existing callers: every
/// field they already read stays exactly where it was, and `_meta`
/// appears alongside.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Envelope<T> {
    #[serde(flatten)]
    pub data: T,
    #[serde(rename = "_meta")]
    pub meta: Meta,
}

impl<T> Envelope<T> {
    pub fn new(data: T, meta: Meta) -> Self {
        Self { data, meta }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::RepoIndex;
    use std::path::PathBuf;

    fn index_with(commit: Option<&str>) -> RepoIndex {
        RepoIndex {
            root: PathBuf::from("/nonexistent"),
            files: Vec::new(),
            other_files: 0,
            indexed_commit: commit.map(str::to_string),
        }
    }

    fn meta(indexed: Option<&str>, live: Option<&str>, age: Option<u64>) -> Meta {
        // Mirrors `Meta::build`'s decision logic without touching git or
        // the filesystem, so the branch table is testable in isolation.
        let indexed_commit = indexed.map(str::to_string);
        let live = live.map(str::to_string);
        let mismatched = match (&indexed_commit, &live) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        };
        Meta {
            timing_ms: 0,
            index_age_days: age,
            indexed_commit: indexed_commit.clone(),
            live_head: if mismatched { live } else { None },
            stale_warning: if mismatched {
                Some("mismatch".to_string())
            } else if indexed_commit.is_none() && age.is_some_and(|d| d >= STALE_AGE_DAYS) {
                Some("unverifiable".to_string())
            } else {
                None
            },
            cached: false,
        }
    }

    #[test]
    fn matching_commit_is_silent() {
        let m = meta(Some("abc123def456"), Some("abc123def456"), Some(0));
        assert_eq!(m.live_head, None, "live_head appears only on a mismatch");
        assert_eq!(m.stale_warning, None);
    }

    #[test]
    fn mismatched_commit_warns_and_reports_live_head() {
        let m = meta(Some("abc123def456"), Some("999888777666"), Some(0));
        assert_eq!(m.live_head.as_deref(), Some("999888777666"));
        assert!(m.stale_warning.is_some());
    }

    /// The case this whole module exists to get right: a fresh index in a
    /// directory with no git must not be reported as stale, and an
    /// unknown commit must not be reported as a match either.
    #[test]
    fn unknown_commit_is_not_a_mismatch() {
        let m = meta(None, None, Some(0));
        assert_eq!(m.stale_warning, None);
        assert_eq!(m.live_head, None);
        assert_eq!(m.indexed_commit, None, "unknown stays unknown");
    }

    /// ...but an unknown commit on an *old* index still gets called out,
    /// worded so it can't be mistaken for a verified mismatch.
    #[test]
    fn old_index_without_commit_is_flagged_as_unverifiable() {
        let m = meta(None, None, Some(STALE_AGE_DAYS));
        assert_eq!(m.stale_warning.as_deref(), Some("unverifiable"));
        assert_eq!(
            m.live_head, None,
            "nothing was compared, so there is no live head to report"
        );
    }

    #[test]
    fn age_just_under_the_threshold_is_silent() {
        let m = meta(None, None, Some(STALE_AGE_DAYS - 1));
        assert_eq!(m.stale_warning, None);
    }

    /// A known indexed commit with git unavailable (live unknown) is
    /// unverifiable, not stale -- and age doesn't override that, because
    /// an index that records its commit is a different situation from
    /// one that doesn't.
    #[test]
    fn known_commit_with_unreadable_head_does_not_warn() {
        let m = meta(Some("abc123def456"), None, Some(STALE_AGE_DAYS * 2));
        assert_eq!(m.stale_warning, None);
        assert_eq!(m.live_head, None);
    }

    #[test]
    fn quiet_fields_are_omitted_from_json() {
        let m = meta(Some("abc123def456"), Some("abc123def456"), Some(3));
        let json = serde_json::to_string(&m).expect("Meta serializes");
        assert!(!json.contains("live_head"), "{json}");
        assert!(!json.contains("stale_warning"), "{json}");
        assert!(!json.contains("cached"), "cached is false: {json}");
        assert!(json.contains("index_age_days"), "{json}");
    }

    #[test]
    fn envelope_keeps_payload_fields_at_top_level() {
        #[derive(Serialize, schemars::JsonSchema)]
        struct Payload {
            file_count: usize,
        }
        let env = Envelope::new(
            Payload { file_count: 7 },
            meta(Some("abc123def456"), Some("abc123def456"), Some(0)),
        );
        let json = serde_json::to_string(&env).expect("Envelope serializes");
        assert!(
            json.contains("\"file_count\":7"),
            "flattening must not nest the payload: {json}"
        );
        assert!(json.contains("\"_meta\""), "{json}");
    }

    #[test]
    fn build_degrades_to_unknown_outside_a_git_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = Meta::build(dir.path(), &index_with(None), 5, false);
        assert_eq!(m.timing_ms, 5);
        assert_eq!(m.indexed_commit, None);
        assert_eq!(m.live_head, None);
        // No index file was ever written here, so age is unknown -- and
        // unknown age must not trip the age-based warning.
        assert_eq!(m.index_age_days, None);
        assert_eq!(m.stale_warning, None);
    }
}
