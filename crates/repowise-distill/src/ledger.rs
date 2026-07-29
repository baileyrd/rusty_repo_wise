//! An append-only record of what distillation actually saved.
//!
//! # Measured, not modelled
//!
//! Every number in this ledger is **observed**: bytes that went into a
//! distillation and bytes that came out, for a command that actually
//! ran. Nothing here is a counterfactual.
//!
//! That matters because the obvious way to inflate a savings report is
//! to estimate what someone *would* have done otherwise. The reference
//! also counts MCP tool responses "against the raw file exploration
//! they replaced" -- a genuinely useful idea, and a genuinely
//! unmeasurable one, since nobody knows what the agent would have read
//! instead. This port does not model that number. Reporting a total
//! that silently mixed measured bytes with a guess would be the same
//! failure this repo has avoided everywhere else (`Option<f64>` for
//! never-measured coverage, `CANNOT ANSWER` in impacted-tests,
//! `UNVERIFIED` in conformance).
//!
//! # Tokens are an approximation, and say so
//!
//! There is no tokenizer here for any particular model. Token counts
//! are bytes divided by [`BYTES_PER_TOKEN`], which is a rule of thumb,
//! and every surface that prints them labels them as approximate. A
//! precise-looking integer derived from a rule of thumb is worse than
//! an obviously rounded one.
//!
//! # Format
//!
//! Tab-separated, one record per line, appended. Deliberately not JSON:
//! this crate has no serde dependency, the schema is four fields wide,
//! and an append-only text file is trivially recoverable by hand if
//! something ever writes a malformed line. Unparseable lines are
//! skipped on read rather than failing the report.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Bytes per token, for the approximate token counts this reports.
///
/// ~4 is the usual rule of thumb for English text and code across
/// common tokenizers. It is not model-specific and is not claimed to
/// be.
pub const BYTES_PER_TOKEN: usize = 4;

pub const LEDGER_FILE: &str = "savings.tsv";

/// What kind of record this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A distillation that ran: measured bytes in and out.
    Distilled,
    /// A command the rewrite hook declined to wrap, with the reason in
    /// `detail`. Feeds `--missed`: this is the feature auditing its own
    /// coverage, the difference between "the hook is working" and "the
    /// hook is installed".
    Skipped,
    /// A command that ran through `distill` but produced no saving.
    /// Carries an exit code, so it counts for fumble detection while
    /// staying out of the savings totals.
    Ran,
}

impl Kind {
    fn tag(&self) -> &'static str {
        match self {
            Kind::Distilled => "distilled",
            Kind::Skipped => "skipped",
            Kind::Ran => "ran",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "distilled" => Some(Kind::Distilled),
            "skipped" => Some(Kind::Skipped),
            "ran" => Some(Kind::Ran),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Unix seconds.
    pub at: u64,
    pub kind: Kind,
    /// The program that ran, or was skipped.
    pub program: String,
    /// Bytes of raw output (`Distilled` only; 0 otherwise).
    pub raw_bytes: usize,
    /// Bytes actually printed (`Distilled` only; 0 otherwise).
    pub kept_bytes: usize,
    /// Skip reason, or empty.
    pub detail: String,
    /// The command's exit status.
    ///
    /// `None` for records that didn't run a command (skips), and for
    /// records written before this field existed -- a trailing field is
    /// absent in older lines, and `read` treats absent as unknown
    /// rather than as success. Conflating "we never saw an exit code"
    /// with "it exited 0" would invent successes that never happened.
    pub exit_code: Option<i32>,
}

impl Record {
    /// Bytes this record saved. Saturating, so a record that somehow
    /// grew can never report a negative saving that would offset a real
    /// one elsewhere in the total.
    pub fn saved_bytes(&self) -> usize {
        self.raw_bytes.saturating_sub(self.kept_bytes)
    }
}

fn sanitize(field: &str) -> String {
    field.replace(['\t', '\n', '\r'], " ")
}

pub fn ledger_path(store_dir: &Path) -> PathBuf {
    store_dir.join(LEDGER_FILE)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append a record.
///
/// Failures are returned but callers are expected to ignore them:
/// accounting must never be able to break the command being wrapped.
pub fn append(store_dir: &Path, record: &Record) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(store_dir)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger_path(store_dir))?;
    writeln!(
        file,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        record.at,
        record.kind.tag(),
        sanitize(&record.program),
        record.raw_bytes,
        record.kept_bytes,
        sanitize(&record.detail),
        record
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".to_string())
    )
}

/// Record a distillation that just happened.
pub fn record_distilled(
    store_dir: &Path,
    program: &str,
    raw_bytes: usize,
    kept_bytes: usize,
    exit_code: i32,
) {
    let _ = append(
        store_dir,
        &Record {
            at: now_secs(),
            kind: Kind::Distilled,
            program: program.to_string(),
            raw_bytes,
            kept_bytes,
            detail: String::new(),
            exit_code: Some(exit_code),
        },
    );
}

/// Record a command that ran through `distill` but wasn't compacted.
///
/// Needed for fumble detection even though there's no saving to report:
/// the *succeeding* half of a fumble pair is usually short output that
/// distillation passed straight through, and without it every fumble
/// would look unresolved.
pub fn record_ran(store_dir: &Path, program: &str, raw_bytes: usize, exit_code: i32) {
    let _ = append(
        store_dir,
        &Record {
            at: now_secs(),
            kind: Kind::Ran,
            program: program.to_string(),
            raw_bytes,
            kept_bytes: raw_bytes,
            detail: String::new(),
            exit_code: Some(exit_code),
        },
    );
}

/// Record a command the rewrite hook declined to wrap.
pub fn record_skipped(store_dir: &Path, program: &str, reason: &str) {
    let _ = append(
        store_dir,
        &Record {
            at: now_secs(),
            kind: Kind::Skipped,
            program: program.to_string(),
            raw_bytes: 0,
            kept_bytes: 0,
            detail: reason.to_string(),
            exit_code: None,
        },
    );
}

/// Read every parseable record. Malformed lines are skipped rather than
/// failing the whole report -- a partially readable ledger is more
/// useful than an error.
pub fn read(store_dir: &Path) -> Vec<Record> {
    let Ok(content) = std::fs::read_to_string(ledger_path(store_dir)) else {
        return Vec::new();
    };
    content.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Record> {
    let mut parts = line.split('\t');
    let at = parts.next()?.parse().ok()?;
    let kind = Kind::parse(parts.next()?)?;
    let program = parts.next()?.to_string();
    let raw_bytes = parts.next()?.parse().ok()?;
    let kept_bytes = parts.next()?.parse().ok()?;
    let detail = parts.next().unwrap_or("").to_string();
    // Absent or "-" is unknown, never success. A record written before
    // this field existed must not be read as having exited 0.
    let exit_code = parts.next().and_then(|s| s.parse().ok());
    Some(Record {
        at,
        kind,
        program,
        raw_bytes,
        kept_bytes,
        detail,
        exit_code,
    })
}

/// Approximate tokens for a byte count. See [`BYTES_PER_TOKEN`].
pub fn approx_tokens(bytes: usize) -> usize {
    bytes / BYTES_PER_TOKEN
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn records_round_trip_through_the_ledger() {
        let d = dir();
        record_distilled(d.path(), "cargo", 4000, 400, 0);
        record_skipped(d.path(), "git", "not-rewritable");

        let records = read(d.path());
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].kind, Kind::Distilled);
        assert_eq!(records[0].program, "cargo");
        assert_eq!(records[0].saved_bytes(), 3600);
        assert_eq!(records[1].kind, Kind::Skipped);
        assert_eq!(records[1].detail, "not-rewritable");
    }

    /// A command containing a tab or newline must not be able to forge
    /// extra fields or extra records.
    #[test]
    fn field_separators_in_a_command_cannot_corrupt_the_ledger() {
        let d = dir();
        record_distilled(d.path(), "car\tgo\nnpm", 100, 10, 0);
        let records = read(d.path());
        assert_eq!(records.len(), 1, "one command must produce one record");
        assert!(!records[0].program.contains('\t'));
        assert!(!records[0].program.contains('\n'));
    }

    #[test]
    fn a_malformed_line_is_skipped_not_fatal() {
        let d = dir();
        record_distilled(d.path(), "cargo", 100, 10, 0);
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(ledger_path(d.path()))
                .unwrap();
            writeln!(f, "garbage line with no fields").unwrap();
        }
        record_distilled(d.path(), "pytest", 200, 20, 0);

        let records = read(d.path());
        assert_eq!(records.len(), 2, "the two good records must still be read");
    }

    /// Saturating rather than signed: a record that somehow grew must
    /// not offset genuine savings elsewhere in a total.
    #[test]
    fn a_record_that_grew_reports_zero_saved_not_a_negative() {
        let r = Record {
            at: 0,
            kind: Kind::Distilled,
            program: "x".into(),
            raw_bytes: 10,
            kept_bytes: 40,
            detail: String::new(),
            exit_code: Some(0),
        };
        assert_eq!(r.saved_bytes(), 0);
    }

    #[test]
    fn reading_a_missing_ledger_is_empty_not_an_error() {
        let d = dir();
        assert!(read(&d.path().join("nope")).is_empty());
    }

    #[test]
    fn token_counts_are_a_byte_ratio() {
        assert_eq!(approx_tokens(4000), 1000);
        assert_eq!(approx_tokens(3), 0);
    }
}
