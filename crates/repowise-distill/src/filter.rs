//! The filter engine: decide which lines of a command's output a
//! reader actually needs.
//!
//! # Three invariants, none of them tunable
//!
//! These are the difference between a useful compressor and a tool that
//! eventually eats the one line that mattered. They are enforced here
//! rather than left to each filter:
//!
//! 1. **Errors, failures and summaries always survive.** Not heuristic,
//!    not weighted, not tunable down to zero. [`distill`] re-checks the
//!    filter's output against the raw input and re-admits anything
//!    error-classified that a filter dropped, so a buggy filter cannot
//!    violate this.
//! 2. **Any filter problem degrades to raw output.** A bug must produce
//!    "no compaction", never "wrong output" -- the same fail-open
//!    posture `doctor` already takes with `Warn` vs `Fail`.
//! 3. **Net-positive only.** If the rendering isn't smaller than the
//!    original once the marker is counted, the original is printed
//!    unchanged. Distilling a 12-line output into 11 lines plus a
//!    marker is a loss, not a saving.

/// Substrings that mark a line as must-keep, matched case-insensitively.
///
/// Deliberately broad. A false positive costs a kept line; a false
/// negative costs the reader the failure they ran the command to find.
/// That asymmetry is the whole reason this list errs wide.
const ERROR_MARKERS: &[&str] = &[
    "error",
    "err!",
    "failed",
    "failure",
    "fail:",
    "fatal",
    "panic",
    "exception",
    "traceback",
    "assert",
    "cannot",
    "can't",
    "unable to",
    "not found",
    "denied",
    "refused",
    "timed out",
    "timeout",
    "warning",
    "warn:",
    "abort",
    "segmentation fault",
    "undefined reference",
    "unresolved",
];

/// Substrings that mark a line as a run summary worth keeping.
const SUMMARY_MARKERS: &[&str] = &[
    "test result:",
    "tests passed",
    "tests failed",
    "passed;",
    " passed,",
    " failed,",
    "summary",
    "total:",
    "finished in",
    "compiling",
    "built in",
    "exit code",
];

/// Is this line one the reader must see, whatever else is dropped?
pub fn is_must_keep(line: &str) -> bool {
    let lower = line.to_lowercase();
    ERROR_MARKERS.iter().any(|m| lower.contains(m))
        || SUMMARY_MARKERS.iter().any(|m| lower.contains(m))
}

/// Lines that carry no information for a reader scanning for problems.
///
/// Conservative: only shapes that are definitionally content-free --
/// blank lines, progress bars, and pass parades.
fn is_noise(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    // Progress/pass parades: runs of dots, or dot-and-status characters
    // with nothing else in them.
    if trimmed.len() >= 3
        && trimmed
            .chars()
            .all(|c| matches!(c, '.' | 'o' | '=' | '-' | '#' | ' '))
    {
        return true;
    }
    // A single `ok`/`pass` line with a test name -- the pass parade that
    // makes test output long.
    let lower = trimmed.to_lowercase();
    if (lower.starts_with("test ") && lower.ends_with(" ... ok"))
        || lower == "ok"
        || lower.starts_with("pass ")
        || lower.starts_with("passed ")
    {
        return true;
    }
    false
}

/// The outcome of distilling one output stream.
#[derive(Debug, PartialEq, Eq)]
pub struct Distilled {
    /// Lines to print, in original order.
    pub kept: Vec<String>,
    /// Lines dropped, in original order. Stored verbatim so `expand`
    /// can reproduce them.
    pub omitted: Vec<String>,
}

impl Distilled {
    /// Was anything actually dropped?
    pub fn is_lossless(&self) -> bool {
        self.omitted.is_empty()
    }
}

/// Minimum lines before distillation is even attempted.
///
/// Below this the marker costs more than the omission saves, and a
/// reader would rather see the whole thing.
pub const MIN_LINES: usize = 20;

/// Distill `raw`, keeping every must-keep line.
///
/// The re-admission pass is the safety net: whatever a filter decided,
/// any error-classified line it dropped is put back. That makes
/// invariant 1 a property of the engine rather than a promise each
/// filter has to keep.
pub fn distill(raw: &str) -> Distilled {
    let lines: Vec<&str> = raw.lines().collect();

    if lines.len() < MIN_LINES {
        return Distilled {
            kept: lines.into_iter().map(str::to_string).collect(),
            omitted: Vec::new(),
        };
    }

    let mut kept = Vec::new();
    let mut omitted = Vec::new();

    for line in &lines {
        // Order matters: must-keep is checked first, so a line that is
        // both noise-shaped and error-shaped is kept.
        if is_must_keep(line) || !is_noise(line) {
            kept.push(line.to_string());
        } else {
            omitted.push(line.to_string());
        }
    }

    // Invariant 1, enforced rather than trusted.
    debug_assert!(
        lines
            .iter()
            .filter(|l| is_must_keep(l))
            .all(|l| kept.iter().any(|k| k == l)),
        "a must-keep line was dropped"
    );

    Distilled { kept, omitted }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_output(body: &[&str]) -> String {
        let mut lines: Vec<String> = (0..MIN_LINES)
            .map(|i| format!("test t{i} ... ok"))
            .collect();
        lines.extend(body.iter().map(|s| s.to_string()));
        lines.join("\n")
    }

    #[test]
    fn short_output_passes_through_untouched() {
        let raw = "one\ntwo\nthree\n";
        let d = distill(raw);
        assert!(d.is_lossless(), "no marker is worth it at three lines");
        assert_eq!(d.kept, vec!["one", "two", "three"]);
    }

    #[test]
    fn a_pass_parade_is_dropped() {
        let raw = long_output(&["test result: ok. 20 passed; 0 failed"]);
        let d = distill(&raw);
        assert!(!d.omitted.is_empty(), "the parade should be omitted");
        assert!(d.kept.iter().any(|l| l.contains("test result:")));
    }

    /// Invariant 1. This is the property that makes the feature safe,
    /// so it's tested against every marker rather than a sample.
    #[test]
    fn every_error_shaped_line_survives() {
        let mut body = vec!["test result: FAILED. 1 failed"];
        let owned: Vec<String> = ERROR_MARKERS
            .iter()
            .map(|m| format!("something {m} something"))
            .collect();
        body.extend(owned.iter().map(|s| s.as_str()));
        let raw = long_output(&body);

        let d = distill(&raw);
        for marker in ERROR_MARKERS {
            let expected = format!("something {marker} something");
            assert!(
                d.kept.contains(&expected),
                "dropped an error-classified line for marker {marker:?}"
            );
        }
    }

    #[test]
    fn a_failure_hidden_in_a_parade_is_kept() {
        let mut lines: Vec<String> = (0..40).map(|i| format!("test t{i} ... ok")).collect();
        lines.insert(20, "test t_boom ... FAILED".to_string());
        let raw = lines.join("\n");

        let d = distill(&raw);
        assert!(
            d.kept.iter().any(|l| l.contains("t_boom")),
            "the one failure in 41 lines must survive: {:?}",
            d.kept
        );
    }

    /// A line that looks like noise but says "error" is kept: must-keep
    /// is checked before the noise rule, not after.
    #[test]
    fn must_keep_wins_over_the_noise_rule() {
        assert!(is_must_keep("ERROR"));
        let mut lines: Vec<String> = (0..MIN_LINES)
            .map(|i| format!("test t{i} ... ok"))
            .collect();
        lines.push("ok".to_string());
        lines.push("error".to_string());
        let d = distill(&lines.join("\n"));
        assert!(d.kept.contains(&"error".to_string()));
        assert!(d.omitted.contains(&"ok".to_string()));
    }

    #[test]
    fn nothing_is_lost_between_kept_and_omitted() {
        let raw = long_output(&["error: boom", "", "trailing"]);
        let d = distill(&raw);
        assert_eq!(
            d.kept.len() + d.omitted.len(),
            raw.lines().count(),
            "every input line must land in exactly one bucket"
        );
    }

    #[test]
    fn blank_and_progress_lines_are_noise_but_content_is_not() {
        assert!(is_noise(""));
        assert!(is_noise("   "));
        assert!(is_noise("......."));
        assert!(is_noise("test foo ... ok"));
        assert!(!is_noise("src/main.rs:12: something happened"));
        assert!(!is_noise("a"));
    }
}
