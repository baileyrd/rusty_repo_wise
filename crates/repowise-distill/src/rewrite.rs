//! Deciding whether a command may be rewritten to run through
//! `repowise distill`.
//!
//! # Why this is a closed set, not a heuristic
//!
//! A rewrite hook sits in the path of *every command an agent runs*.
//! That is far more invasive than the post-commit hook, and the failure
//! mode is worse: a wrong rewrite doesn't just lose output, it changes
//! what executes.
//!
//! So the rule is inverted from most matching problems here. Instead of
//! "rewrite unless it looks dangerous", this is **"never rewrite unless
//! the command is one of a small, named set of shapes we understand"**.
//! Anything unrecognized runs untouched. A missed rewrite costs some
//! tokens; a wrong one costs correctness.
//!
//! # What is never rewritten, regardless of the program
//!
//! Shell metacharacters change what a command *means*, and wrapping
//! `a && b` as `repowise distill a && b` would silently change which
//! part gets distilled -- or worse, what runs at all. Any command
//! containing them bails out entirely rather than being parsed, because
//! a partial understanding of shell syntax is how a wrapper starts
//! executing things nobody typed.

/// Program names whose output is worth distilling and whose invocation
/// shape is simple enough to wrap safely.
///
/// Every entry here is a program that prints a lot and returns a
/// meaningful exit code. Adding to this list is a deliberate act, not a
/// pattern match.
const REWRITABLE: &[&str] = &[
    "cargo",
    "pytest",
    "npm",
    "pnpm",
    "yarn",
    "go",
    "jest",
    "vitest",
    "tsc",
    "eslint",
    "ruff",
    "mypy",
    "flake8",
    "clippy-driver",
    "gradle",
    "mvn",
    "make",
];

/// Characters that mean "this is more than one command", or that redirect
/// or substitute. Their presence disqualifies a command outright.
const SHELL_METACHARACTERS: &[char] = &['|', '&', ';', '>', '<', '`', '$', '(', ')', '\n'];

/// Why a command will or won't be rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Safe to wrap. Carries the program that matched, for reporting.
    Rewrite { program: String },
    /// Left alone, with the reason.
    Skip(SkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Empty or unparseable.
    Empty,
    /// Contains shell metacharacters -- compound, piped, redirected, or
    /// substituted. Never rewritten.
    ShellSyntax,
    /// The program isn't in the closed set.
    NotRewritable,
    /// Already running through distill; wrapping twice would nest
    /// markers inside markers.
    AlreadyDistilled,
}

impl SkipReason {
    pub fn label(&self) -> &'static str {
        match self {
            SkipReason::Empty => "empty",
            SkipReason::ShellSyntax => "shell-syntax",
            SkipReason::NotRewritable => "not-rewritable",
            SkipReason::AlreadyDistilled => "already-distilled",
        }
    }

    pub fn explanation(&self) -> &'static str {
        match self {
            SkipReason::Empty => "nothing to run",
            SkipReason::ShellSyntax => {
                "contains shell metacharacters (pipe, redirect, substitution, or a \
                 compound command) -- rewriting could change what executes, so it never is"
            }
            SkipReason::NotRewritable => {
                "the program isn't in the closed set of shapes this hook understands"
            }
            SkipReason::AlreadyDistilled => "already running through repowise distill",
        }
    }
}

/// Decide whether `command` -- a raw command line -- may be rewritten.
pub fn decide(command: &str) -> Decision {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Decision::Skip(SkipReason::Empty);
    }

    // Checked before anything else. A command containing shell syntax is
    // not parsed at all: understanding it partially is how a wrapper
    // starts running things nobody typed.
    if trimmed.contains(SHELL_METACHARACTERS) {
        return Decision::Skip(SkipReason::ShellSyntax);
    }

    let mut words = trimmed.split_whitespace();
    let Some(first) = words.next() else {
        return Decision::Skip(SkipReason::Empty);
    };

    if first == "repowise" {
        return Decision::Skip(SkipReason::AlreadyDistilled);
    }

    // Compare on the basename so `/usr/bin/cargo` matches, but without
    // letting a path component smuggle something else in.
    let program = first.rsplit('/').next().unwrap_or(first);
    if REWRITABLE.contains(&program) {
        Decision::Rewrite {
            program: program.to_string(),
        }
    } else {
        Decision::Skip(SkipReason::NotRewritable)
    }
}

/// The rewritten command line.
pub fn rewrite(command: &str) -> String {
    format!("repowise distill {}", command.trim())
}

/// Every program this hook will rewrite, for `status` output.
///
/// Printed rather than summarized: someone installing a hook into the
/// path of every command they run is entitled to see the exact list
/// rather than a count.
pub fn rewritable_programs() -> &'static [&'static str] {
    REWRITABLE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewritten(cmd: &str) -> bool {
        matches!(decide(cmd), Decision::Rewrite { .. })
    }

    #[test]
    fn a_plain_recognized_command_is_rewritten() {
        assert!(rewritten("cargo test"));
        assert!(rewritten("pytest -x"));
        assert!(rewritten("npm run build"));
        assert_eq!(
            rewrite("cargo test --workspace"),
            "repowise distill cargo test --workspace"
        );
    }

    #[test]
    fn an_absolute_path_still_matches_on_the_basename() {
        assert!(rewritten("/usr/local/bin/cargo test"));
    }

    #[test]
    fn an_unrecognized_program_is_left_alone() {
        assert_eq!(
            decide("rm -rf /"),
            Decision::Skip(SkipReason::NotRewritable),
            "the closed set is the whole safety model"
        );
        assert_eq!(
            decide("git status"),
            Decision::Skip(SkipReason::NotRewritable)
        );
    }

    /// The rule that matters most. Every one of these could change what
    /// executes if wrapped, so none of them is parsed at all.
    #[test]
    fn anything_with_shell_syntax_is_refused() {
        for cmd in [
            "cargo test | head",
            "cargo test && echo done",
            "cargo test; rm -rf /",
            "cargo test > out.txt",
            "cargo test < in.txt",
            "cargo test `whoami`",
            "cargo test $(whoami)",
            "cargo test $HOME",
            "cargo test\nrm -rf /",
        ] {
            assert_eq!(
                decide(cmd),
                Decision::Skip(SkipReason::ShellSyntax),
                "must refuse to rewrite {cmd:?}"
            );
        }
    }

    /// A rewrite that wrapped an already-wrapped command would nest
    /// markers inside markers.
    #[test]
    fn an_already_distilled_command_is_not_wrapped_twice() {
        assert_eq!(
            decide("repowise distill cargo test"),
            Decision::Skip(SkipReason::AlreadyDistilled)
        );
    }

    #[test]
    fn empty_input_is_reported_as_empty_not_as_unrecognized() {
        assert_eq!(decide(""), Decision::Skip(SkipReason::Empty));
        assert_eq!(decide("   "), Decision::Skip(SkipReason::Empty));
    }

    /// A program whose *name* merely starts with a rewritable one must
    /// not match -- `cargofoo` is not `cargo`.
    #[test]
    fn a_prefix_match_is_not_a_match() {
        assert_eq!(
            decide("cargofoo test"),
            Decision::Skip(SkipReason::NotRewritable)
        );
    }

    #[test]
    fn every_skip_reason_explains_itself() {
        for reason in [
            SkipReason::Empty,
            SkipReason::ShellSyntax,
            SkipReason::NotRewritable,
            SkipReason::AlreadyDistilled,
        ] {
            assert!(!reason.label().is_empty());
            assert!(
                reason.explanation().len() > 10,
                "a skip reason a user might see needs a real explanation"
            );
        }
    }
}
