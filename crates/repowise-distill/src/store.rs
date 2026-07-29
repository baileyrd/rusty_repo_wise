//! The omission store: content-addressed storage for everything a
//! distillation dropped.
//!
//! # Why this exists before the filters do
//!
//! A tool that discards output to save tokens will eventually discard
//! the one line that mattered. What makes distillation defensible
//! rather than dangerous is that it is **reversible**: every dropped
//! byte is stored, and the inline marker naming its ref is a promise
//! that nothing was lost, only moved.
//!
//! That promise has an ordering requirement, which this module exists
//! to enforce: **content is stored before any marker referencing it is
//! rendered.** A marker with nothing behind it is strictly worse than
//! no distillation at all -- it claims recoverability that doesn't
//! exist. So `put` returns the ref only after the bytes are on disk,
//! and a storage failure propagates so the caller can fall back to raw
//! output.
//!
//! # Plain files, not SQLite
//!
//! The reference uses a SQLite sidecar. This port has no SQLite
//! dependency and adding one to store opaque blobs by key would be a
//! large dependency for a small job -- the access pattern here is
//! `put(bytes) -> key` and `get(key) -> bytes`, which is what a
//! filesystem already is. One file per omission under
//! `.repowise/omissions/`, named by its ref.
//!
//! A consequence worth stating: because retrieval is by *filename*, the
//! hash function only has to avoid collisions at write time. It never
//! has to be recomputed to look something up, so it doesn't need to be
//! stable across toolchain versions -- which is what makes
//! `DefaultHasher` (already this repo's content-addressing choice, see
//! `repowise_parser::metrics::body_hash`) sufficient here without
//! pulling in a cryptographic hash crate.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Directory under the repo root (or the user fallback) holding
/// omissions.
pub const STORE_DIR: &str = "omissions";

/// How long an omission stays retrievable.
///
/// 7 days, matching the reference. An agent resuming work tomorrow must
/// still be able to expand yesterday's markers, so this can't be
/// per-session; but a marker from last quarter has no reader left.
pub const TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Total bytes the store may hold before the oldest entries are
/// evicted.
///
/// An unbounded store in a busy repo is a disk-space bug waiting to
/// happen -- distill runs on every wrapped command.
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Characters in a ref. 12 hex digits, matching the reference's marker
/// format so the two are visually interchangeable.
const REF_LEN: usize = 12;

pub struct Store {
    dir: PathBuf,
}

/// Where a store lives for `start`.
///
/// Inside a repowise repo, alongside the index. Otherwise a user-level
/// fallback, so `repowise distill` still works (and stays reversible)
/// when run somewhere that was never indexed -- refusing there would
/// make the wrapper useless in exactly the ad-hoc cases it's handy for.
pub fn store_dir(start: &Path, home: Option<&Path>) -> PathBuf {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        if dir.join(".repowise").is_dir() {
            return dir.join(".repowise").join(STORE_DIR);
        }
        cur = dir.parent();
    }
    match home {
        Some(h) => h.join(".repowise").join(STORE_DIR),
        None => start.join(".repowise").join(STORE_DIR),
    }
}

fn hash_ref(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    let full = format!("{:016x}", hasher.finish());
    full.chars().take(REF_LEN).collect()
}

/// A ref is well-formed if it's exactly [`REF_LEN`] lowercase hex
/// digits. Checked on read so a caller-supplied string can never
/// escape the store directory.
pub fn is_valid_ref(r: &str) -> bool {
    r.len() == REF_LEN && r.chars().all(|c| c.is_ascii_hexdigit())
}

/// Pull a ref out of a pasted marker, or accept a bare ref.
///
/// Both forms are accepted because someone will copy the whole
/// `[repowise#...]` string out of their scrollback rather than
/// carefully selecting the 12 characters inside it.
pub fn parse_ref(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if is_valid_ref(trimmed) {
        return Some(trimmed.to_string());
    }
    let after = trimmed.split("repowise#").nth(1)?;
    let candidate: String = after
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .take(REF_LEN)
        .collect();
    is_valid_ref(&candidate).then_some(candidate)
}

/// Why a `get` returned nothing.
///
/// The distinction is the whole point: "you mistyped it" and "the store
/// discarded it" call for different responses from the reader, and a
/// bare "not found" would send them down the wrong path.
#[derive(Debug, PartialEq, Eq)]
pub enum Missing {
    /// The ref isn't 12 hex digits.
    Malformed,
    /// Well-formed, but nothing is stored under it. Either it was never
    /// written, or pruning evicted it.
    NotStored,
}

impl Store {
    pub fn open(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, r: &str) -> PathBuf {
        self.dir.join(format!("{r}.txt"))
    }

    /// Store `content` and return its ref.
    ///
    /// Errors propagate rather than being swallowed: the caller must
    /// fall back to printing raw output, because a marker whose content
    /// wasn't stored is a broken promise.
    pub fn put(&self, content: &str) -> anyhow::Result<String> {
        std::fs::create_dir_all(&self.dir)?;
        let mut r = hash_ref(content);

        // Collision handling. A 48-bit ref collides rarely, but a
        // collision would silently return *someone else's output* --
        // the one failure mode worse than losing the content, since
        // the reader has no way to notice. So an occupied ref holding
        // different bytes gets rehashed rather than overwritten.
        let mut attempt = 0u32;
        loop {
            let path = self.path_for(&r);
            match std::fs::read_to_string(&path) {
                Ok(existing) if existing == content => return Ok(r),
                Ok(_) => {
                    attempt += 1;
                    r = hash_ref(&format!("{content}\u{0}{attempt}"));
                    if attempt > 64 {
                        anyhow::bail!("could not find a free omission ref after 64 attempts");
                    }
                }
                Err(_) => break,
            }
        }

        std::fs::write(self.path_for(&r), content)?;
        // Opportunistic, and deliberately after the write: pruning must
        // never be able to evict the entry whose marker is about to be
        // printed.
        let _ = self.prune(&r);
        Ok(r)
    }

    /// Retrieve stored content.
    pub fn get(&self, r: &str) -> Result<String, Missing> {
        if !is_valid_ref(r) {
            return Err(Missing::Malformed);
        }
        std::fs::read_to_string(self.path_for(r)).map_err(|_| Missing::NotStored)
    }

    /// Drop entries past the TTL, then oldest-first until under the
    /// size cap.
    ///
    /// `keep` is never evicted -- it's the entry whose marker is about
    /// to be rendered, and a just-printed marker that already dangles
    /// would be the most confusing possible outcome.
    pub fn prune(&self, keep: &str) -> anyhow::Result<usize> {
        let mut entries: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.file_stem().and_then(|s| s.to_str()) == Some(keep) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push((path, modified, meta.len()));
        }

        let now = SystemTime::now();
        let mut removed = 0usize;
        entries.retain(|(path, modified, _)| {
            let expired = now
                .duration_since(*modified)
                .map(|age| age > TTL)
                .unwrap_or(false);
            if expired && std::fs::remove_file(path).is_ok() {
                removed += 1;
                return false;
            }
            true
        });

        let mut total: u64 = entries.iter().map(|(_, _, len)| len).sum();
        if total > MAX_BYTES {
            entries.sort_by_key(|(_, modified, _)| *modified);
            for (path, _, len) in &entries {
                if total <= MAX_BYTES {
                    break;
                }
                if std::fs::remove_file(path).is_ok() {
                    total = total.saturating_sub(*len);
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    /// Total bytes currently held. Used by `repowise doctor`.
    pub fn size_bytes(&self) -> u64 {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        entries
            .flatten()
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("omissions"));
        (dir, store)
    }

    #[test]
    fn put_then_get_round_trips_byte_for_byte() {
        let (_d, store) = store();
        let content = "line one\nline two\n\ttabbed\n\nblank above\n";
        let r = store.put(content).unwrap();
        assert_eq!(store.get(&r).unwrap(), content);
    }

    #[test]
    fn identical_content_reuses_one_ref() {
        let (_d, store) = store();
        let a = store.put("same").unwrap();
        let b = store.put("same").unwrap();
        assert_eq!(a, b, "content addressing shouldn't duplicate");
    }

    #[test]
    fn refs_are_twelve_hex_digits() {
        let (_d, store) = store();
        let r = store.put("whatever").unwrap();
        assert!(is_valid_ref(&r), "{r}");
    }

    /// A malformed ref and an evicted one need different responses from
    /// the reader, so they're different errors.
    #[test]
    fn missing_distinguishes_malformed_from_evicted() {
        let (_d, store) = store();
        store.put("x").unwrap();
        assert_eq!(store.get("not-a-ref").unwrap_err(), Missing::Malformed);
        assert_eq!(store.get("aaaaaaaaaaaa").unwrap_err(), Missing::NotStored);
    }

    /// A ref is used to build a path, so it must not be able to walk
    /// out of the store directory.
    #[test]
    fn a_traversal_attempt_is_rejected_as_malformed() {
        let (_d, store) = store();
        assert_eq!(store.get("../../etc/pw").unwrap_err(), Missing::Malformed);
        assert_eq!(store.get("..").unwrap_err(), Missing::Malformed);
    }

    #[test]
    fn parse_ref_accepts_a_bare_ref_and_a_pasted_marker() {
        assert_eq!(parse_ref("a1b2c3d4e5f6").as_deref(), Some("a1b2c3d4e5f6"));
        assert_eq!(
            parse_ref("[repowise#a1b2c3d4e5f6: 230 lines omitted]").as_deref(),
            Some("a1b2c3d4e5f6")
        );
        assert_eq!(
            parse_ref("  repowise#a1b2c3d4e5f6  ").as_deref(),
            Some("a1b2c3d4e5f6")
        );
        assert_eq!(parse_ref("nonsense"), None);
    }

    /// The entry whose marker is about to be printed must survive its
    /// own write, whatever the size cap says.
    #[test]
    fn prune_never_evicts_the_entry_being_kept() {
        let (_d, store) = store();
        let r = store.put("fresh content").unwrap();
        store.prune(&r).unwrap();
        assert_eq!(store.get(&r).unwrap(), "fresh content");
    }

    #[test]
    fn store_dir_prefers_an_enclosing_repowise_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".repowise")).unwrap();
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();

        assert_eq!(
            store_dir(&nested, None),
            root.join(".repowise").join(STORE_DIR),
            "should walk up to the repo rather than writing a store in a subdirectory"
        );
    }

    #[test]
    fn store_dir_falls_back_to_home_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();

        assert_eq!(
            store_dir(&elsewhere, Some(&home)),
            home.join(".repowise").join(STORE_DIR)
        );
    }

    #[test]
    fn size_bytes_reports_zero_for_a_store_that_was_never_written() {
        let (_d, store) = store();
        assert_eq!(store.size_bytes(), 0);
    }
}
