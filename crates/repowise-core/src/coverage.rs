//! Test-coverage ingest — the foundation of the test-intelligence layer
//! (issue #241).
//!
//! Two data shapes are kept, matching the reference repowise's model:
//!
//! - a **per-file aggregate**, merged across every test, which drives
//!   coverage-gap analysis and the health markers in #243; and
//! - a **per-test map**, recording which test covered which lines, which
//!   is what `repowise impacted-tests` (#242) needs.
//!
//! The per-test map is recorded now even though its only consumer lands
//! later: re-ingesting every report a second time just to add it would
//! be wasted work, and LCOV gives it to us for free via `TN:`.
//!
//! **LCOV only, deliberately.** The reference also reads Cobertura XML,
//! Clover XML, coverage.py's SQLite files, and its own normalized JSON.
//! LCOV is the one format that needs no new dependency to parse — every
//! other listed format would pull in an XML or SQLite crate. The rest
//! are follow-ups, not omissions.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Ingested coverage for one repository.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoverageData {
    /// `path -> line -> execution count`, summed across every record and
    /// every test. A line present with a count of `0` is *known to be
    /// uncovered*, which is a different fact from a line that's absent
    /// because no report ever mentioned it.
    pub files: BTreeMap<PathBuf, BTreeMap<usize, usize>>,
    /// `test name -> path -> lines that test executed`. Empty when the
    /// ingested reports carried no `TN:` records.
    pub per_test: BTreeMap<String, BTreeMap<PathBuf, BTreeSet<usize>>>,
}

/// What `ingest` did with the paths it saw. Reported rather than
/// discarded: a report whose paths don't line up with the index produces
/// perfect-looking-but-empty coverage, and silently swallowing that
/// would make the whole layer quietly useless.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestSummary {
    pub files_ingested: usize,
    pub tests_seen: usize,
    /// Source paths from the report that don't exist under the repo
    /// root.
    pub unmatched_paths: Vec<PathBuf>,
}

impl CoverageData {
    pub const COVERAGE_FILE: &'static str = "coverage.json";

    pub fn coverage_path(root: &Path) -> PathBuf {
        root.join(crate::RepoIndex::INDEX_DIR)
            .join(Self::COVERAGE_FILE)
    }

    pub fn save(&self, root: &Path) -> anyhow::Result<PathBuf> {
        let dir = root.join(crate::RepoIndex::INDEX_DIR);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(Self::COVERAGE_FILE);
        serde_json::to_writer_pretty(std::fs::File::create(&path)?, self)?;
        Ok(path)
    }

    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = Self::coverage_path(root);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("no coverage data at {}: {e}", path.display()))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Fold `other` into `self`. Execution counts add; per-test line sets
    /// union. Merging rather than replacing is what lets `coverage add`
    /// take several reports (one per test suite, or one per CI shard).
    pub fn merge(&mut self, other: CoverageData) {
        for (path, lines) in other.files {
            let entry = self.files.entry(path).or_default();
            for (line, count) in lines {
                *entry.entry(line).or_insert(0) += count;
            }
        }
        for (test, files) in other.per_test {
            let entry = self.per_test.entry(test).or_default();
            for (path, lines) in files {
                entry.entry(path).or_default().extend(lines);
            }
        }
    }

    /// Fraction of this file's *known* lines that were executed at least
    /// once, `0.0..=100.0`.
    ///
    /// `None` when the file appears in no report at all — deliberately
    /// distinct from `Some(0.0)`, which means "reports covered this file
    /// and none of its lines ran". Callers that conflate the two would
    /// report untested files that simply weren't measured.
    pub fn line_coverage_of(&self, path: &Path) -> Option<f64> {
        let lines = self.files.get(path)?;
        if lines.is_empty() {
            return None;
        }
        let hit = lines.values().filter(|c| **c > 0).count();
        Some(hit as f64 / lines.len() as f64 * 100.0)
    }

    /// Names of tests known to have executed at least one line of
    /// `path`.
    pub fn tests_covering(&self, path: &Path) -> Vec<&str> {
        self.per_test
            .iter()
            .filter(|(_, files)| files.get(path).is_some_and(|l| !l.is_empty()))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Whether any per-test data was recorded. `impacted-tests` needs
    /// this to tell "no test touches these lines" apart from "no per-test
    /// map was ever ingested" — see #242.
    pub fn has_per_test_map(&self) -> bool {
        !self.per_test.is_empty()
    }
}

/// Parse one LCOV report.
///
/// Handles the records that carry the information this layer needs:
/// `TN:` (test name), `SF:` (source file), `DA:<line>,<count>`, and
/// `end_of_record`. Summary records (`LF:`/`LH:`/`BR*`/`FN*`) are
/// ignored rather than rejected -- they're derivable from `DA:` and
/// their absence in older writers shouldn't fail an ingest.
///
/// Paths are returned exactly as the report spells them; resolving them
/// against a repo root is [`ingest`]'s job.
pub fn parse_lcov(text: &str) -> anyhow::Result<CoverageData> {
    let mut data = CoverageData::default();
    let mut test_name: Option<String> = None;
    let mut current: Option<PathBuf> = None;

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("TN:") {
            // An empty TN: is the conventional "no test context" marker
            // and must not become a test literally named "".
            test_name = Some(rest.trim().to_string()).filter(|s| !s.is_empty());
        } else if let Some(rest) = line.strip_prefix("SF:") {
            current = Some(PathBuf::from(rest.trim()));
            data.files.entry(PathBuf::from(rest.trim())).or_default();
        } else if let Some(rest) = line.strip_prefix("DA:") {
            let Some(path) = current.clone() else {
                anyhow::bail!("line {}: DA: record before any SF: record", lineno + 1);
            };
            let (line_no, count) = parse_da(rest, lineno + 1)?;
            *data
                .files
                .entry(path.clone())
                .or_default()
                .entry(line_no)
                .or_insert(0) += count;

            if count > 0 {
                if let Some(test) = &test_name {
                    data.per_test
                        .entry(test.clone())
                        .or_default()
                        .entry(path)
                        .or_default()
                        .insert(line_no);
                }
            }
        } else if line == "end_of_record" {
            current = None;
        }
        // Everything else (LF:, LH:, FN*, BR*, comments) is ignored.
    }

    Ok(data)
}

fn parse_da(rest: &str, lineno: usize) -> anyhow::Result<(usize, usize)> {
    let mut parts = rest.trim().split(',');
    let line_no = parts
        .next()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .ok_or_else(|| anyhow::anyhow!("line {lineno}: malformed DA: record {rest:?}"))?;
    // Some writers emit a checksum as a third field; ignore it.
    let count = parts
        .next()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .ok_or_else(|| anyhow::anyhow!("line {lineno}: malformed DA: record {rest:?}"))?;
    Ok((line_no, count))
}

/// Parse `text` and rewrite its source paths to absolute paths under
/// `root`, dropping any that don't resolve to a real file.
///
/// LCOV paths may be absolute (from the machine that ran the suite) or
/// relative to some build directory, so neither form can be trusted
/// as-is. Anything that doesn't land on an existing file under `root` is
/// returned in [`IngestSummary::unmatched_paths`] rather than silently
/// dropped: coverage that looks fine but matches nothing is the most
/// likely failure mode of this whole layer.
pub fn ingest(text: &str, root: &Path) -> anyhow::Result<(CoverageData, IngestSummary)> {
    let parsed = parse_lcov(text)?;
    let mut out = CoverageData::default();
    let mut summary = IngestSummary::default();
    let mut unmatched = BTreeSet::new();

    let mut resolve = |p: &Path| -> Option<PathBuf> {
        let candidate = if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        };
        if candidate.is_file() {
            return Some(candidate.canonicalize().unwrap_or(candidate));
        }
        // Absolute path from another machine: retry its tail against the
        // root, longest suffix first, so `/ci/build/src/lib.rs` still
        // finds `<root>/src/lib.rs`.
        let components: Vec<_> = p.components().collect();
        for start in 0..components.len() {
            let suffix: PathBuf = components[start..].iter().collect();
            let candidate = root.join(&suffix);
            if candidate.is_file() {
                return Some(candidate.canonicalize().unwrap_or(candidate));
            }
        }
        unmatched.insert(p.to_path_buf());
        None
    };

    for (path, lines) in parsed.files {
        if let Some(resolved) = resolve(&path) {
            out.files.entry(resolved).or_default().extend(lines);
        }
    }
    for (test, files) in parsed.per_test {
        for (path, lines) in files {
            if let Some(resolved) = resolve(&path) {
                out.per_test
                    .entry(test.clone())
                    .or_default()
                    .entry(resolved)
                    .or_default()
                    .extend(lines);
            }
        }
    }

    summary.files_ingested = out.files.len();
    summary.tests_seen = out.per_test.len();
    summary.unmatched_paths = unmatched.into_iter().collect();
    Ok((out, summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = "\
TN:my_test
SF:src/lib.rs
DA:1,3
DA:2,0
DA:3,1
LF:3
LH:2
end_of_record
";

    #[test]
    fn parses_lines_and_counts() {
        let data = parse_lcov(SIMPLE).unwrap();
        let lines = &data.files[Path::new("src/lib.rs")];
        assert_eq!(lines[&1], 3);
        assert_eq!(lines[&2], 0);
        assert_eq!(lines[&3], 1);
    }

    #[test]
    fn records_the_per_test_map_from_tn_records() {
        let data = parse_lcov(SIMPLE).unwrap();
        assert!(data.has_per_test_map());
        let covered = &data.per_test["my_test"][Path::new("src/lib.rs")];
        // Only executed lines belong to a test's map -- line 2 ran zero times.
        assert!(covered.contains(&1));
        assert!(covered.contains(&3));
        assert!(!covered.contains(&2));
    }

    #[test]
    fn an_empty_tn_is_not_a_test_named_empty_string() {
        let data = parse_lcov("TN:\nSF:src/lib.rs\nDA:1,1\nend_of_record\n").unwrap();
        assert!(!data.has_per_test_map(), "{:?}", data.per_test);
    }

    #[test]
    fn coverage_percentage_counts_hit_lines_over_known_lines() {
        let data = parse_lcov(SIMPLE).unwrap();
        let pct = data.line_coverage_of(Path::new("src/lib.rs")).unwrap();
        assert!((pct - 66.666).abs() < 0.01, "{pct}");
    }

    #[test]
    fn an_unmeasured_file_is_none_not_zero_percent() {
        // "never measured" and "measured, nothing ran" must not collapse
        // into the same answer -- a caller would report the first as
        // untested.
        let data = parse_lcov(SIMPLE).unwrap();
        assert_eq!(data.line_coverage_of(Path::new("src/other.rs")), None);

        let zero = parse_lcov("SF:src/zero.rs\nDA:1,0\nend_of_record\n").unwrap();
        assert_eq!(zero.line_coverage_of(Path::new("src/zero.rs")), Some(0.0));
    }

    #[test]
    fn merging_two_reports_sums_counts_and_unions_test_maps() {
        let mut a = parse_lcov(SIMPLE).unwrap();
        let b = parse_lcov("TN:other_test\nSF:src/lib.rs\nDA:2,5\nend_of_record\n").unwrap();
        a.merge(b);

        let lines = &a.files[Path::new("src/lib.rs")];
        assert_eq!(lines[&2], 5, "0 + 5");
        assert_eq!(lines[&1], 3, "untouched by the second report");

        let mut tests = a.tests_covering(Path::new("src/lib.rs"));
        tests.sort();
        assert_eq!(tests, vec!["my_test", "other_test"]);
    }

    #[test]
    fn ignores_summary_records_rather_than_rejecting_them() {
        let with_extras = "SF:src/lib.rs\nFN:1,foo\nFNDA:2,foo\nBRDA:1,0,0,1\nDA:1,1\nLF:1\nLH:1\nend_of_record\n";
        let data = parse_lcov(with_extras).unwrap();
        assert_eq!(data.files[Path::new("src/lib.rs")][&1], 1);
    }

    #[test]
    fn tolerates_a_checksum_third_field_on_da_records() {
        let data = parse_lcov("SF:src/lib.rs\nDA:7,2,abc123hash\nend_of_record\n").unwrap();
        assert_eq!(data.files[Path::new("src/lib.rs")][&7], 2);
    }

    #[test]
    fn malformed_input_errors_rather_than_panicking() {
        assert!(parse_lcov("SF:src/lib.rs\nDA:notanumber,1\nend_of_record\n").is_err());
        assert!(parse_lcov("SF:src/lib.rs\nDA:1\nend_of_record\n").is_err());
        // A DA: with no preceding SF: has nowhere to attach.
        assert!(parse_lcov("DA:1,1\n").is_err());
    }

    #[test]
    fn ingest_reports_paths_that_match_nothing_instead_of_dropping_them() {
        let root = std::env::temp_dir().join("repowise-coverage-test-unmatched");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("lib.rs"), "fn a() {}\n").unwrap();

        let report = "SF:src/lib.rs\nDA:1,1\nend_of_record\n\
                      SF:src/ghost.rs\nDA:1,1\nend_of_record\n";
        let (data, summary) = ingest(report, &root).unwrap();

        assert_eq!(summary.files_ingested, 1);
        assert_eq!(summary.unmatched_paths, vec![PathBuf::from("src/ghost.rs")]);
        assert!(data.files.keys().any(|p| p.ends_with("src/lib.rs")));
    }

    #[test]
    fn ingest_resolves_an_absolute_path_recorded_on_another_machine() {
        // A CI runner writes /builds/proj/src/lib.rs; locally that file
        // lives at <root>/src/lib.rs. Matching by suffix recovers it.
        let root = std::env::temp_dir().join("repowise-coverage-test-abs");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("lib.rs"), "fn a() {}\n").unwrap();

        let report = "SF:/builds/proj/src/lib.rs\nDA:1,1\nend_of_record\n";
        let (data, summary) = ingest(report, &root).unwrap();

        assert_eq!(summary.files_ingested, 1);
        assert!(summary.unmatched_paths.is_empty(), "{summary:?}");
        assert!(data.files.keys().any(|p| p.ends_with("src/lib.rs")));
    }
}
