//! Recurring command fumbles: the same program run twice, the first
//! failing and a later variant succeeding.
//!
//! # Why this reads the ledger and not agent transcripts
//!
//! The obvious source is a coding agent's session transcript, and this
//! feature was originally specified against Claude Code's JSONL. That
//! was measured against a real 582-command transcript before any code
//! was written, and it does not work:
//!
//! - Only **16 of 581** commands were detectable as failed by any
//!   available signal -- a ~2.7% recall ceiling. `is_error` fires for
//!   permission rejections and MCP errors, not command failures, and a
//!   nonzero exit is not recorded at all.
//! - Grouping by "base command" degenerates, because nearly every real
//!   command is a compound starting with `cd`. Running the pairing
//!   anyway produced matches like `grep ...` "corrected by" `ls ...`,
//!   which share only that prefix.
//!
//! A tool built on that would emit confident-looking noise into a
//! managed block in `CLAUDE.md`, where it would be read as guidance.
//!
//! `repowise distill` *runs* the command, so it knows the exit code
//! exactly. Nothing here is inferred. The cost is that only commands
//! run after the rewrite hook is installed are observed -- there is no
//! retroactive history, which is why the report is explicit that an
//! empty result means "nothing observed", not "no fumbles".

use crate::ledger::{Kind, Record};
use std::collections::BTreeMap;

/// How many later runs count as "a later variant" of a failed command.
///
/// Small on purpose. A correction follows its fumble closely; pairing
/// across a wider window starts matching unrelated invocations that
/// merely share a program, which is the failure mode that sank the
/// transcript approach.
pub const WINDOW: usize = 3;

/// One recurring fumble.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fumble {
    pub program: String,
    /// How many times a failing run of this program was followed by a
    /// succeeding one.
    pub count: usize,
    /// Distinct exit codes seen on the failing runs, sorted. Reported
    /// because "always exit 2" and "a different code each time" are
    /// different problems.
    pub exit_codes: Vec<i32>,
}

/// Did this record run a command that failed?
///
/// `None` exit codes are **not** failures and **not** successes -- they
/// are unknown, and are excluded from both sides of a pair rather than
/// assumed either way.
fn failed(r: &Record) -> bool {
    matches!(r.kind, Kind::Distilled | Kind::Ran) && r.exit_code.is_some_and(|c| c != 0)
}

fn succeeded(r: &Record) -> bool {
    matches!(r.kind, Kind::Distilled | Kind::Ran) && r.exit_code == Some(0)
}

/// Find fumbles occurring at least `min_count` times.
///
/// Records must be in chronological order, which is what the
/// append-only ledger guarantees.
pub fn detect(records: &[Record], min_count: usize) -> Vec<Fumble> {
    let mut counts: BTreeMap<String, (usize, Vec<i32>)> = BTreeMap::new();

    for (i, record) in records.iter().enumerate() {
        if !failed(record) {
            continue;
        }
        // Look only a short way ahead, and only at the same program.
        let corrected = records
            .iter()
            .skip(i + 1)
            .filter(|r| matches!(r.kind, Kind::Distilled | Kind::Ran))
            .take(WINDOW)
            .any(|r| r.program == record.program && succeeded(r));

        if corrected {
            let entry = counts
                .entry(record.program.clone())
                .or_insert((0, Vec::new()));
            entry.0 += 1;
            if let Some(code) = record.exit_code {
                if !entry.1.contains(&code) {
                    entry.1.push(code);
                }
            }
        }
    }

    let mut out: Vec<Fumble> = counts
        .into_iter()
        .filter(|(_, (count, _))| *count >= min_count)
        .map(|(program, (count, mut exit_codes))| {
            exit_codes.sort_unstable();
            Fumble {
                program,
                count,
                exit_codes,
            }
        })
        .collect();
    out.sort_by_key(|f| std::cmp::Reverse(f.count));
    out
}

/// How many ledger records carried a usable exit status.
///
/// Reported alongside the findings so a thin result can be read
/// correctly: few observations means little was watched, which is not
/// the same as few fumbles.
pub fn observed_runs(records: &[Record]) -> usize {
    records
        .iter()
        .filter(|r| matches!(r.kind, Kind::Distilled | Kind::Ran) && r.exit_code.is_some())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(program: &str, exit: Option<i32>) -> Record {
        Record {
            at: 0,
            kind: Kind::Ran,
            program: program.to_string(),
            raw_bytes: 10,
            kept_bytes: 10,
            detail: String::new(),
            exit_code: exit,
        }
    }

    fn skip(program: &str) -> Record {
        Record {
            at: 0,
            kind: Kind::Skipped,
            program: program.to_string(),
            raw_bytes: 0,
            kept_bytes: 0,
            detail: "not-rewritable".into(),
            exit_code: None,
        }
    }

    #[test]
    fn a_failure_followed_by_a_success_is_a_fumble() {
        let records = vec![run("cargo", Some(101)), run("cargo", Some(0))];
        let found = detect(&records, 1);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].program, "cargo");
        assert_eq!(found[0].count, 1);
        assert_eq!(found[0].exit_codes, vec![101]);
    }

    #[test]
    fn a_failure_never_corrected_is_not_a_fumble() {
        let records = vec![run("cargo", Some(101)), run("cargo", Some(1))];
        assert!(
            detect(&records, 1).is_empty(),
            "two failures in a row is a broken build, not a fumble someone corrected"
        );
    }

    #[test]
    fn a_different_program_does_not_correct_it() {
        let records = vec![run("cargo", Some(1)), run("pytest", Some(0))];
        assert!(detect(&records, 1).is_empty());
    }

    /// The window is what stops unrelated later invocations from being
    /// read as corrections -- the exact failure that sank transcript
    /// mining.
    #[test]
    fn a_success_far_later_does_not_count_as_the_correction() {
        let mut records = vec![run("cargo", Some(1))];
        for _ in 0..WINDOW {
            records.push(run("npm", Some(0)));
        }
        records.push(run("cargo", Some(0)));
        assert!(
            detect(&records, 1).is_empty(),
            "a success {} runs later is not a correction",
            WINDOW + 1
        );
    }

    /// Unknown exit status must not be read as either outcome.
    #[test]
    fn an_unknown_exit_code_is_neither_a_failure_nor_a_correction() {
        assert!(detect(&[run("cargo", None), run("cargo", Some(0))], 1).is_empty());
        assert!(detect(&[run("cargo", Some(1)), run("cargo", None)], 1).is_empty());
    }

    #[test]
    fn skipped_records_are_ignored_entirely() {
        let records = vec![skip("cargo"), run("cargo", Some(1)), run("cargo", Some(0))];
        let found = detect(&records, 1);
        assert_eq!(found.len(), 1, "a skip carries no exit status to pair on");
    }

    #[test]
    fn min_count_gates_one_off_fumbles() {
        let records = vec![run("cargo", Some(1)), run("cargo", Some(0))];
        assert!(detect(&records, 2).is_empty());
        assert_eq!(detect(&records, 1).len(), 1);
    }

    #[test]
    fn distinct_exit_codes_are_reported_separately() {
        let records = vec![
            run("cargo", Some(101)),
            run("cargo", Some(0)),
            run("cargo", Some(2)),
            run("cargo", Some(0)),
        ];
        let found = detect(&records, 1);
        assert_eq!(found[0].count, 2);
        assert_eq!(
            found[0].exit_codes,
            vec![2, 101],
            "'always the same code' and 'a different one each time' are different problems"
        );
    }

    #[test]
    fn observed_runs_counts_only_records_with_a_known_exit_status() {
        let records = vec![run("a", Some(0)), run("b", None), skip("c")];
        assert_eq!(observed_runs(&records), 1);
    }
}
