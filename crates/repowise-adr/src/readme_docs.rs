//! README/docs-prose decision mining (issue #71): decision-flavored
//! sections of `README.md`/`ARCHITECTURE.md`, e.g. a "## Why we chose
//! sled over rocksdb" heading followed by prose that reads like a
//! decision.
//!
//! By far the noisiest of this crate's nine sources — a README describes
//! the *system*, not (mostly) the choices behind it, so a naive
//! "does this paragraph contain a decision keyword" scan over the whole
//! file would flag ordinary descriptive prose constantly. Two mitigations
//! apply, mirroring how the real repowise handles this same source (see
//! `docs/layers/DECISIONS.md` upstream):
//!
//! 1. **Structural gate.** Only text under a Markdown heading counts, and
//!    only when that heading's own section (see `heading_level` below for
//!    how a section's extent is found) contains decision-keyword prose —
//!    the same `is_decision_message` heuristic `commits.rs`/
//!    `pull_requests.rs` use. This is a much smaller net than scanning
//!    every paragraph in the file: an ordinary "## Installation" or
//!    "## Usage" section's prose essentially never trips the keyword
//!    heuristic, while a "## Why" or "## Design decisions" section's
//!    prose reliably does.
//! 2. **Confidence, not exclusion.** Unlike a heading-title allowlist
//!    (which would require *knowing* the right heading names up front,
//!    and silently miss a real decision under an unanticipated one), this
//!    source doesn't restrict by heading text at all — it relies on
//!    `crate::confidence_for` to rank every survivor at 3, tied with the
//!    freeform `CodeComment` source and below every source that's either
//!    an explicit opt-in convention (`InlineMarker`) or a dedicated
//!    decision artifact (`Adr`, `Changelog`, `CommitMessage`,
//!    `PullRequest`, `Manual`). A reader (or a caller sorting by
//!    confidence) sees README-mined decisions for what they are: the
//!    least-trustworthy written-artifact source, not a false-positive
//!    filtered down to zero.

use crate::commits::is_decision_message;
use crate::{DecisionRecord, DecisionSource};
use std::path::Path;

/// Checked independently (unlike `changelog.rs`'s first-match-wins
/// `find_changelog`) — a repo can reasonably have both a README and an
/// ARCHITECTURE doc, each with its own "why" sections, so both are mined
/// when present rather than picking one.
const README_FILENAMES: &[&str] = &["readme.md", "architecture.md"];

/// Mine decision-flavored prose sections from whichever of
/// `README_FILENAMES` exist at `root` (case-insensitive filename match).
/// Neither file existing degrades to an empty result, the same
/// "not required" tradeoff every other source in this crate makes.
pub fn mine_readme_decisions(root: &Path) -> Vec<DecisionRecord> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let entries: Vec<_> = entries.flatten().collect();

    let mut records = Vec::new();
    for candidate in README_FILENAMES {
        let Some(entry) = entries.iter().find(|e| {
            e.path().is_file() && e.file_name().to_string_lossy().to_lowercase() == *candidate
        }) else {
            continue;
        };
        let path = entry.path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        records.extend(parse_sections(&path, &rel, &text));
    }
    records
}

/// Split `text` into heading-delimited sections and keep the ones whose
/// body reads as decision-like. A section runs from one ATX heading
/// (`# ` through `###### `) to the *next heading of any level*, the same
/// flat boundary `changelog.rs`'s `parse_changelog` uses for its
/// `### `-only sections, generalized to every heading depth here.
/// Deliberately flat rather than nesting-aware: a "## Why" section's
/// "### The tradeoff" subsection is checked (and, if decision-like,
/// mined) as its own independent section rather than folding its text
/// into the parent's — nesting a level-1 section until the next level-1
/// heading would let a README's single top-of-file `# Title` (with no
/// sibling `#` heading anywhere else in the file) swallow every section
/// below it into one body.
fn parse_sections(path: &Path, rel: &Path, text: &str) -> Vec<DecisionRecord> {
    let lines: Vec<&str> = text.lines().collect();
    let mut records = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if heading_level(lines[i]).is_none() {
            i += 1;
            continue;
        };
        let heading = lines[i]
            .trim_start()
            .trim_start_matches('#')
            .trim()
            .to_string();
        let start_line = i + 1;

        let mut body_lines = Vec::new();
        let mut j = i + 1;
        while j < lines.len() && heading_level(lines[j]).is_none() {
            let line = lines[j].trim();
            if !line.is_empty() {
                body_lines.push(line);
            }
            j += 1;
        }
        let body = body_lines.join("\n");

        if !heading.is_empty() && is_decision_message(&body) {
            records.push(DecisionRecord {
                linked_files: Vec::new(),
                ..DecisionRecord::new(
                    format!("readme:{}:{start_line}", rel.display()),
                    format!(
                        "{heading}: {}",
                        body_lines.first().copied().unwrap_or_default()
                    ),
                    DecisionSource::ReadmeMining {
                        file: path.to_path_buf(),
                        line: start_line,
                        heading,
                    },
                    body,
                )
            });
        }

        i = j;
    }

    records
}

/// `Some(level)` (1-6) if `line` is an ATX Markdown heading (`#` through
/// `######`, followed by a space and at least some title text);
/// otherwise `None`. Requires the space so a `#`-prefixed shell comment
/// or a stray `#tag` in prose isn't mistaken for a heading.
fn heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.starts_with(' ') || rest.trim().is_empty() {
        return None;
    }
    Some(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).unwrap();
    }

    #[test]
    fn mines_a_decision_like_section_from_readme() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(
            &root,
            "README.md",
            "# My Project\n\n\
             ## Installation\n\
             Run `cargo install`.\n\n\
             ## Why sled\n\
             We decided to adopt sled over rocksdb for the index store.\n\n\
             ## Usage\n\
             See the CLI help.\n",
        );

        let records = mine_readme_decisions(&root);

        assert_eq!(records.len(), 1, "{records:?}");
        assert!(records[0].title.starts_with("Why sled:"));
        assert!(matches!(
            &records[0].source,
            DecisionSource::ReadmeMining { heading, line: 6, .. } if heading == "Why sled"
        ));
    }

    #[test]
    fn ignores_ordinary_descriptive_sections() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(
            &root,
            "README.md",
            "# My Project\n\n## Installation\nRun `cargo install`.\n\n## Usage\nSee the CLI help.\n",
        );

        assert!(mine_readme_decisions(&root).is_empty());
    }

    #[test]
    fn a_subsection_with_its_own_decision_prose_is_mined_independently() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(
            &root,
            "README.md",
            "## Why\n\
             Context up top, nothing notable.\n\n\
             ### The tradeoff\n\
             We chose sled instead of rocksdb here.\n\n\
             ## Next section\n\
             Unrelated prose.\n",
        );

        let records = mine_readme_decisions(&root);

        // Flat section boundaries (see `parse_sections`'s doc comment):
        // "## Why"'s own body has no decision keyword and isn't mined;
        // its "### The tradeoff" subsection is checked -- and mined --
        // independently, not folded into "## Why"'s body.
        assert_eq!(records.len(), 1, "{records:?}");
        assert!(matches!(
            &records[0].source,
            DecisionSource::ReadmeMining { heading, .. } if heading == "The tradeoff"
        ));
        assert!(records[0].body.contains("sled instead of rocksdb"));
        assert!(!records[0].body.contains("nothing notable"));
        assert!(!records[0].body.contains("Unrelated prose"));
    }

    #[test]
    fn mines_both_readme_and_architecture_when_both_are_present() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(
            &root,
            "README.md",
            "## Why\nWe decided to adopt sled over rocksdb.\n",
        );
        write(
            &root,
            "ARCHITECTURE.md",
            "## Rationale\nWe chose an event-sourced design instead of CRUD.\n",
        );

        let records = mine_readme_decisions(&root);

        assert_eq!(records.len(), 2, "{records:?}");
    }

    #[test]
    fn readme_mined_decisions_carry_the_lowest_non_inferred_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(
            &root,
            "README.md",
            "## Why\nWe decided to adopt sled over rocksdb.\n",
        );

        let records = mine_readme_decisions(&root);

        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].confidence,
            crate::confidence_for(&records[0].source)
        );
        // Tied with CodeComment (rank 3), the crate's other freeform-prose
        // source -- see the module doc for why.
        assert_eq!(
            records[0].confidence,
            crate::confidence_for(&DecisionSource::CodeComment {
                file: PathBuf::from("x"),
                line: 1,
            })
        );
    }

    #[test]
    fn returns_empty_when_neither_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(mine_readme_decisions(&root).is_empty());
    }

    #[test]
    fn a_hash_prefixed_line_without_a_following_space_is_not_a_heading() {
        assert_eq!(heading_level("#!/usr/bin/env bash"), None);
        assert_eq!(heading_level("#no-space"), None);
        assert_eq!(heading_level("## Why"), Some(2));
    }
}
