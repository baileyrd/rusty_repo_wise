//! CHANGELOG-based decision mining: `CHANGELOG.md`/`HISTORY.md`/
//! `NEWS.md`/`CHANGES.md` at the repo root, specifically the
//! keep-a-changelog-style `### Changed`/`### Removed`/`### Deprecated`/
//! `### Security` sections — `### Added`/`### Fixed` are deliberately
//! excluded, since purely additive or bug-fix entries aren't
//! architectural decisions the way a change/removal/deprecation/security
//! call generally is.
//!
//! A heading-text match, not a full keep-a-changelog spec parser (per
//! this issue's own acceptance criteria) — pure filesystem/parsing, no
//! new dependency. Unlike the PR-body/code-comment/inline-marker
//! sources, a changelog entry's `linked_files` goes through the same
//! text-matching linker ADR files and commit messages use (in `mine()`)
//! rather than an authoritative self-link: the changelog file itself
//! isn't what the decision is *about*, unlike a PR's diff or the file a
//! comment sits in.

use crate::{DecisionRecord, DecisionSource};
use std::path::{Path, PathBuf};

/// Checked in this priority order (case-insensitive) so the result is
/// deterministic if more than one happens to exist.
const CHANGELOG_FILENAMES: &[&str] = &["changelog.md", "history.md", "news.md", "changes.md"];

/// Section headings treated as decision-like. Case-insensitive exact
/// match against the text after a `### ` heading marker.
const DECISION_SECTIONS: &[&str] = &["Changed", "Removed", "Deprecated", "Security"];

/// Mine decision-like sections from whichever changelog file is found
/// at `root` first (see `CHANGELOG_FILENAMES`'s priority order). No
/// changelog file at all degrades to an empty result, same "not
/// required" tradeoff every other source in this crate makes.
pub fn mine_changelog_decisions(root: &Path) -> Vec<DecisionRecord> {
    let Some(path) = find_changelog(root) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
    parse_changelog(&path, &rel, &text)
}

fn find_changelog(root: &Path) -> Option<PathBuf> {
    let entries: Vec<_> = std::fs::read_dir(root).ok()?.flatten().collect();
    for candidate in CHANGELOG_FILENAMES {
        if let Some(entry) = entries.iter().find(|e| {
            e.path().is_file() && e.file_name().to_string_lossy().to_lowercase() == *candidate
        }) {
            return Some(entry.path());
        }
    }
    None
}

fn parse_changelog(path: &Path, rel: &Path, text: &str) -> Vec<DecisionRecord> {
    let lines: Vec<&str> = text.lines().collect();
    let mut records = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let Some(heading) = lines[i].trim().strip_prefix("### ") else {
            i += 1;
            continue;
        };
        let heading = heading.trim();
        let Some(&section) = DECISION_SECTIONS
            .iter()
            .find(|s| s.eq_ignore_ascii_case(heading))
        else {
            i += 1;
            continue;
        };

        let start_line = i + 1;
        let mut body_lines = Vec::new();
        let mut j = i + 1;
        while j < lines.len() && !lines[j].trim_start().starts_with('#') {
            let line = lines[j].trim();
            if !line.is_empty() {
                body_lines.push(line);
            }
            j += 1;
        }
        let body = body_lines.join("\n");

        records.push(DecisionRecord {
            id: format!("changelog:{}:{start_line}", rel.display()),
            title: format!(
                "{section}: {}",
                body_lines.first().copied().unwrap_or_default()
            ),
            source: DecisionSource::Changelog {
                file: path.to_path_buf(),
                section: section.to_string(),
            },
            status: None,
            superseded_by: None,
            date: None,
            body,
            linked_files: Vec::new(),
        });

        i = j;
    }

    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).unwrap();
    }

    #[test]
    fn mines_each_recognized_section_from_a_keep_a_changelog_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(
            &root,
            "CHANGELOG.md",
            "# Changelog\n\n\
             ## [1.0.0] - 2026-01-01\n\
             ### Added\n\
             - A new widget.\n\n\
             ### Changed\n\
             - Switched the index store from rocksdb to sled.\n\n\
             ### Deprecated\n\
             - The old `foo` API.\n\n\
             ### Removed\n\
             - The legacy config format.\n\n\
             ### Security\n\
             - Patched a path-traversal bug.\n\n\
             ### Fixed\n\
             - A crash on empty input.\n",
        );

        let records = mine_changelog_decisions(&root);
        let sections: Vec<&str> = records
            .iter()
            .map(|r| match &r.source {
                DecisionSource::Changelog { section, .. } => section.as_str(),
                _ => unreachable!(),
            })
            .collect();

        assert_eq!(
            sections,
            vec!["Changed", "Deprecated", "Removed", "Security"]
        );
        let changed = records
            .iter()
            .find(|r| matches!(&r.source, DecisionSource::Changelog { section, .. } if section == "Changed"))
            .unwrap();
        assert!(changed.body.contains("rocksdb to sled"));
    }

    #[test]
    fn finds_changelog_case_insensitively_and_prefers_it_over_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(&root, "changelog.MD", "### Changed\n- from changelog\n");
        write(&root, "HISTORY.md", "### Changed\n- from history\n");

        let records = mine_changelog_decisions(&root);
        assert_eq!(records.len(), 1);
        assert!(records[0].body.contains("from changelog"));
    }

    #[test]
    fn falls_back_to_history_when_no_changelog_is_present() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(&root, "HISTORY.md", "### Removed\n- an old thing\n");

        let records = mine_changelog_decisions(&root);
        assert_eq!(records.len(), 1);
        assert!(matches!(
            &records[0].source,
            DecisionSource::Changelog { section, .. } if section == "Removed"
        ));
    }

    #[test]
    fn returns_empty_when_no_changelog_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(mine_changelog_decisions(&root).is_empty());
    }

    #[test]
    fn ignores_non_decision_sections_like_added_and_fixed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(
            &root,
            "CHANGELOG.md",
            "### Added\n- a thing\n\n### Fixed\n- a bug\n",
        );

        assert!(mine_changelog_decisions(&root).is_empty());
    }
}
