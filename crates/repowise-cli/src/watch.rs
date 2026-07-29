//! `repowise watch` -- debounced file-watch re-indexing.
//!
//! # The bug this module is mostly about
//!
//! A re-indexer that watches the directory it writes into will trigger
//! on **its own writes** and loop forever. It is silent -- nothing
//! errors, the index just churns and a core stays pegged -- so it gets
//! noticed by the fan, not the output. [`should_reindex`] exists to be
//! testable in isolation for exactly that reason.
//!
//! `.repowise/` is where the index is written. `.git/` churns on every
//! git command and contains nothing this tool indexes. Both are
//! excluded before debouncing, not after, so they can't even reset the
//! timer.
//!
//! # Deterministic only
//!
//! The reference's `watch` regenerates LLM wiki prose. This drives
//! `update` -- the same re-index the post-commit hook runs. That keeps
//! `watch` in the deterministic tier the rest of this port lives in,
//! and means it costs nothing to leave running.
//!
//! # Why a watcher that dies must be loud
//!
//! inotify has per-user watch limits that a large repo can exhaust.
//! When that happens the process is still alive and still looks like
//! it's working, while the index quietly goes stale -- the worst
//! outcome, because the user believes staleness is impossible while
//! this runs. So watcher errors terminate the command with a nonzero
//! exit and an explanation, rather than being logged and swallowed.

use std::path::Path;
use std::time::Duration;

/// Default quiet period after the last change before re-indexing.
///
/// 2s, matching the reference. A single editor save emits several
/// events and build tools emit thousands; re-indexing per event would
/// make the machine unusable. This is the one tuning knob that actually
/// matters, so it's a flag.
pub const DEFAULT_DEBOUNCE_MS: u64 = 2000;

/// Directory names whose contents never warrant a re-index.
///
/// `.repowise` is the self-trigger loop. `.git` churns constantly (every
/// `git status` touches it) and holds nothing indexable.
const IGNORED_DIRS: &[&str] = &[".repowise", ".git"];

/// Should a change at `path` trigger a re-index?
///
/// Compared against `root` so the check is about the path's position in
/// the repo, not about substrings that might appear anywhere in an
/// absolute path -- a repo that happens to live under `/home/.git-old/`
/// must still be watchable.
pub fn should_reindex(path: &Path, root: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    !relative
        .components()
        .any(|c| IGNORED_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()))
}

/// Is this event kind a *content* change worth re-indexing for?
///
/// This is the second half of the self-trigger problem, and the half
/// that path filtering alone does not solve.
///
/// Re-indexing **reads every file in the repo**, and on Linux inotify
/// reports each read as an `Access` event for a perfectly ordinary
/// source path. Accepting those means the re-index re-triggers itself
/// through files it only looked at — a loop that no `.repowise`/`.git`
/// exclusion can catch, because the paths involved are exactly the
/// paths we care about. It was found by running the watcher and
/// watching it re-index forever while idle.
///
/// Metadata-only changes are excluded for the same reason at lower
/// volume: a `chmod` or an atime bump changes nothing this tool
/// indexes.
pub fn is_content_change(kind: &notify::EventKind) -> bool {
    use notify::event::{EventKind, ModifyKind};
    match kind {
        EventKind::Create(_) | EventKind::Remove(_) => true,
        EventKind::Modify(ModifyKind::Metadata(_)) => false,
        EventKind::Modify(_) => true,
        // Reads. The loop.
        EventKind::Access(_) => false,
        // `Any`/`Other` are backend-specific catch-alls. Treated as
        // real changes: missing an edit is worse than one extra
        // re-index, and unlike `Access` these don't fire on reads.
        EventKind::Any | EventKind::Other => true,
    }
}

pub fn debounce_duration(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from("/repo")
    }

    /// The loop. Without this the watcher re-indexes forever off its
    /// own output, silently.
    #[test]
    fn writes_into_the_index_directory_never_trigger_a_reindex() {
        assert!(!should_reindex(
            &root().join(".repowise/index.json"),
            &root()
        ));
        assert!(!should_reindex(
            &root().join(".repowise/omissions/abc.txt"),
            &root()
        ));
        assert!(!should_reindex(&root().join(".repowise"), &root()));
    }

    #[test]
    fn git_internals_never_trigger_a_reindex() {
        assert!(!should_reindex(&root().join(".git/index"), &root()));
        assert!(!should_reindex(
            &root().join(".git/refs/heads/main"),
            &root()
        ));
    }

    #[test]
    fn ordinary_source_changes_do_trigger() {
        assert!(should_reindex(&root().join("src/lib.rs"), &root()));
        assert!(should_reindex(
            &root().join("crates/a/src/main.rs"),
            &root()
        ));
        assert!(should_reindex(&root().join("README.md"), &root()));
    }

    /// Matching on components rather than substrings: a repo living
    /// under a path that merely contains `.git` must still be watchable.
    #[test]
    fn an_ignored_name_above_the_root_does_not_disable_watching() {
        let root = PathBuf::from("/home/user/.git-backups/myrepo");
        assert!(
            should_reindex(&root.join("src/lib.rs"), &root),
            "the ignore rule is about position inside the repo, not substrings"
        );
    }

    /// A file merely *named* like an ignored directory is still source.
    #[test]
    fn a_file_named_like_an_ignored_dir_is_not_confused_for_one() {
        assert!(should_reindex(&root().join("src/.gitignore"), &root()));
        assert!(should_reindex(&root().join("docs/.repowise.md"), &root()));
    }

    #[test]
    fn a_path_outside_the_root_is_still_checked_by_component() {
        let other = PathBuf::from("/elsewhere/.git/config");
        assert!(
            !should_reindex(&other, &root()),
            "an unrelated path with an ignored component is still ignored, \
             not accidentally admitted"
        );
    }

    #[test]
    fn debounce_converts_milliseconds() {
        assert_eq!(debounce_duration(2000), Duration::from_secs(2));
    }

    /// The bug this was written for: re-indexing reads every file, and
    /// inotify reports reads as Access events on ordinary source paths.
    /// Path filtering can't catch that -- the paths are exactly the ones
    /// we watch -- so the event kind has to.
    #[test]
    fn reads_never_count_as_changes() {
        use notify::event::{AccessKind, AccessMode, EventKind};
        assert!(!is_content_change(&EventKind::Access(AccessKind::Read)));
        assert!(!is_content_change(&EventKind::Access(AccessKind::Open(
            AccessMode::Read
        ))));
        assert!(!is_content_change(&EventKind::Access(AccessKind::Any)));
    }

    #[test]
    fn writes_creates_and_removes_count_as_changes() {
        use notify::event::{CreateKind, DataChange, EventKind, ModifyKind, RemoveKind};
        assert!(is_content_change(&EventKind::Create(CreateKind::File)));
        assert!(is_content_change(&EventKind::Remove(RemoveKind::File)));
        assert!(is_content_change(&EventKind::Modify(ModifyKind::Data(
            DataChange::Content
        ))));
    }

    #[test]
    fn metadata_only_changes_do_not_count() {
        use notify::event::{EventKind, MetadataKind, ModifyKind};
        assert!(!is_content_change(&EventKind::Modify(
            ModifyKind::Metadata(MetadataKind::AccessTime)
        )));
        assert!(!is_content_change(&EventKind::Modify(
            ModifyKind::Metadata(MetadataKind::Permissions)
        )));
    }

    /// Backend catch-alls are treated as real: missing an edit is worse
    /// than one extra re-index, and unlike Access they don't fire on
    /// reads.
    #[test]
    fn ambiguous_event_kinds_are_treated_as_changes() {
        use notify::event::EventKind;
        assert!(is_content_change(&EventKind::Any));
        assert!(is_content_change(&EventKind::Other));
    }
}
