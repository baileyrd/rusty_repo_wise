//! `repowise impacted-tests` — which tests a change provably exercises
//! (issue #242).
//!
//! # The safeguard this module exists to enforce
//!
//! **An empty test list must never read as "nothing to test."** Absence
//! of evidence and evidence of absence have to stay distinguishable, so
//! every result carries an explicit [`Status`] rather than just a
//! (possibly empty) list.
//!
//! Getting this wrong is actively dangerous, not merely unhelpful: a
//! developer who reads "no impacted tests" as "safe to skip testing",
//! when the real cause was "no coverage was ever ingested", has been
//! misled by the tool into shipping untested code. Every path below
//! that *cannot* answer says so in words.

use repowise_core::coverage::CoverageData;
use repowise_git::ChangedLines;
use std::path::Path;

/// Why the impacted-test list looks the way it does. Mirrors the
/// discriminators the reference documents (`map_present`, `no_map`,
/// `no_index`, `unknown`, `no_source_line_changes`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// A per-test map exists and was intersected against the diff. Only
    /// in this state does an empty list mean "no test covers these
    /// lines".
    MapPresent,
    /// Coverage exists but carries no per-test contexts (no `TN:`
    /// records), so which test covered which line is unknowable.
    NoMap,
    /// No coverage has been ingested at all.
    NoCoverage,
    /// The revspec resolved, but touched no lines of any measured source
    /// file -- e.g. a docs-only or whitespace-only change.
    NoSourceLineChanges,
}

impl Status {
    /// Whether an empty `tests` list in this state is a real finding
    /// rather than a gap in the data. Only `MapPresent` qualifies.
    pub fn empty_list_is_meaningful(&self) -> bool {
        matches!(self, Status::MapPresent)
    }
}

#[derive(Debug)]
pub struct Impacted {
    pub status: Status,
    /// Test names known to execute at least one changed line, sorted.
    pub tests: Vec<String>,
    pub files_changed: usize,
    pub lines_changed: usize,
}

/// Intersect a diff's changed lines with the per-test coverage map.
pub fn select(changed: &ChangedLines, coverage: Option<&CoverageData>) -> Impacted {
    let files_changed = changed.len();
    let lines_changed: usize = changed.values().map(|l| l.len()).sum();

    let Some(coverage) = coverage else {
        return Impacted {
            status: Status::NoCoverage,
            tests: Vec::new(),
            files_changed,
            lines_changed,
        };
    };
    if !coverage.has_per_test_map() {
        return Impacted {
            status: Status::NoMap,
            tests: Vec::new(),
            files_changed,
            lines_changed,
        };
    }

    // Only files the coverage actually measured can contribute. A diff
    // touching only unmeasured files is "no source line changes" from
    // this layer's point of view, not "no tests" -- reporting the latter
    // would be the exact misreading this module guards against.
    let touched_measured: usize = changed
        .iter()
        .filter(|(path, _)| coverage.files.contains_key(*path))
        .map(|(_, lines)| lines.len())
        .sum();
    if touched_measured == 0 {
        return Impacted {
            status: Status::NoSourceLineChanges,
            tests: Vec::new(),
            files_changed,
            lines_changed,
        };
    }

    let mut tests: Vec<String> = coverage
        .per_test
        .iter()
        .filter(|(_, covered)| {
            covered.iter().any(|(path, lines)| {
                changed
                    .get(path)
                    .is_some_and(|ch| lines.iter().any(|l| ch.contains(l)))
            })
        })
        .map(|(name, _)| name.clone())
        .collect();
    tests.sort();

    Impacted {
        status: Status::MapPresent,
        tests,
        files_changed,
        lines_changed,
    }
}

pub fn render(result: &Impacted, revspec: &str, root: &Path) -> String {
    let mut out = format!("Impacted tests for {revspec} in {}\n", root.display());
    out.push_str(&format!(
        "  diff: {} file(s), {} changed line(s)\n",
        result.files_changed, result.lines_changed
    ));

    let (line, explanation) = match &result.status {
        Status::NoCoverage => (
            "no coverage data ingested",
            "this is not the same as \"no tests are affected\".\n\
             \x20 Run `repowise coverage add <REPORT>` first.",
        ),
        Status::NoMap => (
            "coverage present, but no per-test map",
            "the ingested reports carried no TN: records, so which test\n\
             \x20 covered which line is unknown. Re-run your suite with per-test\n\
             \x20 contexts enabled.",
        ),
        // An entirely empty diff is worth calling out separately: `git
        // show` reports no changes at all for a merge commit, which is
        // easy to mistake for "this change touched nothing".
        Status::NoSourceLineChanges if result.files_changed == 0 => (
            "the revspec resolved to an empty diff",
            "no files changed. Note that `git show` reports no diff for a\n\
             \x20 merge commit -- try one of its parents, or a `base..head` range.",
        ),
        Status::NoSourceLineChanges => (
            "no changed lines in any measured file",
            "the diff touched only files coverage never measured (docs-only,\n\
             \x20 or unmeasured sources).",
        ),
        Status::MapPresent => ("per-test map consulted", ""),
    };
    out.push_str(&format!("  status: {line}\n"));

    // One place decides whether an empty list is a finding or a gap in
    // the data -- the distinction this whole module exists to preserve.
    if result.tests.is_empty() && !result.status.empty_list_is_meaningful() {
        out.push_str(&format!("  CANNOT ANSWER -- {explanation}\n"));
        out.push_str("  An empty list here is NOT evidence that nothing is affected.\n");
        return out;
    }

    if result.tests.is_empty() {
        out.push_str("  No test in the map executes any of the changed lines.\n");
        out.push_str(
            "  Note: that means untested by the *ingested* suite -- not that the\n\
             \x20 change is safe.\n",
        );
        return out;
    }

    out.push_str(&format!("  {} impacted test(s):\n", result.tests.len()));
    for t in &result.tests {
        out.push_str(&format!("    {t}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::coverage::parse_lcov;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    fn changed(path: &str, lines: &[usize]) -> ChangedLines {
        let mut m: ChangedLines = BTreeMap::new();
        m.insert(
            PathBuf::from(path),
            lines.iter().copied().collect::<BTreeSet<_>>(),
        );
        m
    }

    fn coverage() -> CoverageData {
        parse_lcov(
            "TN:test_a\nSF:src/lib.rs\nDA:10,1\nDA:11,1\nend_of_record\n\
             TN:test_b\nSF:src/lib.rs\nDA:50,1\nend_of_record\n",
        )
        .unwrap()
    }

    #[test]
    fn selects_only_tests_that_touch_the_changed_lines() {
        let result = select(&changed("src/lib.rs", &[10]), Some(&coverage()));
        assert_eq!(result.status, Status::MapPresent);
        assert_eq!(result.tests, vec!["test_a"]);
    }

    #[test]
    fn an_empty_list_is_only_meaningful_when_the_map_was_consulted() {
        // The whole point of the module: these three states all yield an
        // empty list, and only one of them means "no test is affected".
        let no_cov = select(&changed("src/lib.rs", &[10]), None);
        assert!(no_cov.tests.is_empty());
        assert!(!no_cov.status.empty_list_is_meaningful());

        let no_map = select(
            &changed("src/lib.rs", &[10]),
            Some(&parse_lcov("SF:src/lib.rs\nDA:10,1\nend_of_record\n").unwrap()),
        );
        assert_eq!(no_map.status, Status::NoMap);
        assert!(!no_map.status.empty_list_is_meaningful());

        // Changed a line no test covers, but the map WAS consulted.
        let real = select(&changed("src/lib.rs", &[11]), Some(&coverage()));
        assert_eq!(real.status, Status::MapPresent);
        assert_eq!(real.tests, vec!["test_a"]);
    }

    #[test]
    fn a_change_to_an_unmeasured_file_is_not_reported_as_no_tests() {
        let result = select(&changed("docs/README.md", &[1]), Some(&coverage()));
        assert_eq!(result.status, Status::NoSourceLineChanges);
        assert!(!result.status.empty_list_is_meaningful());
    }

    #[test]
    fn a_measured_line_no_test_covers_yields_an_honest_empty_list() {
        // Line 99 is in a measured file but in no test's map.
        let mut cov = coverage();
        cov.files
            .entry(PathBuf::from("src/lib.rs"))
            .or_default()
            .insert(99, 0);
        let result = select(&changed("src/lib.rs", &[99]), Some(&cov));
        assert_eq!(result.status, Status::MapPresent);
        assert!(result.tests.is_empty());
        assert!(result.status.empty_list_is_meaningful());
    }

    #[test]
    fn every_cannot_answer_state_says_so_in_the_output() {
        for (result, needle) in [
            (select(&changed("src/lib.rs", &[10]), None), "CANNOT ANSWER"),
            (
                select(
                    &changed("src/lib.rs", &[10]),
                    Some(&parse_lcov("SF:src/lib.rs\nDA:10,1\nend_of_record\n").unwrap()),
                ),
                "CANNOT ANSWER",
            ),
            (
                select(&changed("docs/x.md", &[1]), Some(&coverage())),
                "NOT evidence",
            ),
        ] {
            let out = render(&result, "HEAD", Path::new("/repo"));
            assert!(out.contains(needle), "{out}");
        }
    }

    #[test]
    fn a_genuine_empty_result_still_warns_against_reading_it_as_safe() {
        let mut cov = coverage();
        cov.files
            .entry(PathBuf::from("src/lib.rs"))
            .or_default()
            .insert(99, 0);
        let out = render(
            &select(&changed("src/lib.rs", &[99]), Some(&cov)),
            "HEAD",
            Path::new("/repo"),
        );
        assert!(out.contains("not that the"), "{out}");
        assert!(out.contains("ingested"), "{out}");
    }

    #[test]
    fn an_empty_diff_names_the_merge_commit_trap() {
        // `git show` reports nothing for a merge commit; without this
        // the output reads as "this change touched nothing".
        let result = Impacted {
            status: Status::NoSourceLineChanges,
            tests: Vec::new(),
            files_changed: 0,
            lines_changed: 0,
        };
        let out = render(&result, "HEAD", Path::new("/repo"));
        assert!(out.contains("merge commit"), "{out}");
        assert!(out.contains("empty diff"), "{out}");
    }
}
