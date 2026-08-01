//! Deterministic, signature-based secret/credential detection over
//! indexed source files (issue #360).
//!
//! # Scope, and why it stops here
//!
//! Upstream's own docs describe security scanning as "a separate
//! security layer" -- an "adjacent axis" to Code Health, not folded into
//! it -- and its own tool-comparison table concedes "full SAST/SCA/
//! secrets/IaC" scanning to dedicated tools rather than claiming it
//! itself. No dedicated security-layer doc exists alongside upstream's
//! `CODE_HEALTH.md`/`CHANGE_RISK.md`/etc., and its dashboard only
//! mentions "the security findings table, by directory and by
//! severity" as a one-line sub-tab description -- there's no public
//! specification of what it actually detects.
//!
//! Given that, this crate covers exactly one category, chosen because
//! it's deterministic, well-understood, and needs no infrastructure
//! this port doesn't already have:
//!
//! - **Hardcoded secrets/credentials**: known-prefix signatures (AWS
//!   access key IDs, GitHub/Slack tokens, PEM private-key blocks) plus
//!   one generic "suspicious assignment" heuristic, filtered against a
//!   placeholder denylist to keep obvious non-secrets out.
//!
//! Two categories the issue itself raised are deliberately **not**
//! covered here:
//!
//! - **Dependency CVE checking** needs a live vulnerability feed --
//!   this port has no infrastructure for one, and a hardcoded snapshot
//!   would go stale the day it shipped. That's a different problem than
//!   this port's static, point-in-time analysis model solves; see
//!   `repowise-external-deps` for the declared-dependency inventory
//!   this would need to build on if ever attempted.
//! - **Insecure-pattern/injection-shape detection** (SQL injection
//!   shape, etc.) needs real dataflow/taint analysis to keep false
//!   positives down at any real precision -- exactly the ground
//!   upstream's own comparison table concedes to dedicated SAST tools.
//!   A regex-only approximation here would be noisy enough to not be
//!   worth shipping.
//!
//! Findings never include the matched secret text itself, in
//! [`SecurityFinding::message`] or anywhere else -- a report is not the
//! place to re-leak what it found.

use regex::Regex;
use repowise_core::RepoIndex;
use std::path::PathBuf;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityFindingKind {
    AwsAccessKeyId,
    PrivateKeyBlock,
    GitHubToken,
    SlackToken,
    SuspiciousAssignment,
}

impl SecurityFindingKind {
    pub fn label(&self) -> &'static str {
        match self {
            SecurityFindingKind::AwsAccessKeyId => "aws-access-key-id",
            SecurityFindingKind::PrivateKeyBlock => "private-key-block",
            SecurityFindingKind::GitHubToken => "github-token",
            SecurityFindingKind::SlackToken => "slack-token",
            SecurityFindingKind::SuspiciousAssignment => "suspicious-assignment",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecurityFinding {
    pub file: PathBuf,
    pub line: usize,
    pub kind: SecurityFindingKind,
    pub severity: Severity,
    /// A human description of *what pattern matched*, never the matched
    /// text itself -- see this module's own doc comment.
    pub message: String,
}

struct Signature {
    kind: SecurityFindingKind,
    severity: Severity,
    pattern: &'static LazyLock<Regex>,
    message: &'static str,
}

static AWS_ACCESS_KEY_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap());

static PRIVATE_KEY_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN (RSA |EC |OPENSSH |DSA |PGP )?PRIVATE KEY-----").unwrap()
});

static GITHUB_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{36}\b").unwrap());

static SLACK_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bxox[baprs]-[A-Za-z0-9-]{10,48}\b").unwrap());

/// `key = "value"` / `key: "value"` (quotes required, so this doesn't
/// fire on bare identifiers or environment-variable *names*) where
/// `key` looks credential-shaped and `value` is long enough to plausibly
/// be a real secret rather than a short flag or enum tag.
static SUSPICIOUS_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(api[_-]?key|secret[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret|private[_-]?key|password|passwd)\s*[:=]\s*["']([^"'\s]{16,})["']"#,
    )
    .unwrap()
});

fn signatures() -> [Signature; 4] {
    [
        Signature {
            kind: SecurityFindingKind::AwsAccessKeyId,
            severity: Severity::High,
            pattern: &AWS_ACCESS_KEY_ID,
            message: "an AWS access key ID literal",
        },
        Signature {
            kind: SecurityFindingKind::PrivateKeyBlock,
            severity: Severity::High,
            pattern: &PRIVATE_KEY_BLOCK,
            message: "a PEM private-key block",
        },
        Signature {
            kind: SecurityFindingKind::GitHubToken,
            severity: Severity::High,
            pattern: &GITHUB_TOKEN,
            message: "a GitHub access token literal",
        },
        Signature {
            kind: SecurityFindingKind::SlackToken,
            severity: Severity::High,
            pattern: &SLACK_TOKEN,
            message: "a Slack token literal",
        },
    ]
}

/// Values that look like a secret to [`SUSPICIOUS_ASSIGNMENT`] but
/// obviously aren't one -- placeholders, examples, and test fixtures.
/// Checked case-insensitively as a substring, not an exact match, since
/// placeholder values are rarely written consistently
/// (`"your-api-key-here"`, `"YOUR_API_KEY"`, `"<api-key>"`, ...).
const PLACEHOLDER_MARKERS: &[&str] = &[
    "example",
    "changeme",
    "change_me",
    "your_",
    "your-",
    "xxx",
    "todo",
    "fixme",
    "placeholder",
    "dummy",
    "sample",
    "insert_",
    "replace_",
    "<",
    "fake",
    "test_secret",
    "notasecret",
    "not-a-secret",
];

fn looks_like_a_placeholder(value: &str) -> bool {
    let lower = value.to_lowercase();
    if PLACEHOLDER_MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    // All one repeated character ("xxxxxxxxxxxxxxxx", "0000000000000000")
    // carries no entropy -- never a real secret.
    value.chars().all(|c| c == value.chars().next().unwrap())
}

/// Scan every indexed file's current on-disk content. Read errors (a
/// file the index still lists but that's since been deleted, or one
/// that isn't valid UTF-8) are skipped rather than failing the whole
/// scan -- consistent with every other best-effort report in this port
/// (`repowise_distill::ledger::read`'s malformed-line handling,
/// `repowise-docs`'s freshness check).
pub fn scan(index: &RepoIndex) -> Vec<SecurityFinding> {
    let signatures = signatures();
    let mut findings = Vec::new();

    for file in &index.files {
        let Ok(content) = std::fs::read_to_string(&file.path) else {
            continue;
        };

        for (i, line) in content.lines().enumerate() {
            let line_no = i + 1;

            for sig in &signatures {
                if sig.pattern.is_match(line) {
                    findings.push(SecurityFinding {
                        file: file.path.clone(),
                        line: line_no,
                        kind: sig.kind,
                        severity: sig.severity,
                        message: sig.message.to_string(),
                    });
                }
            }

            if let Some(caps) = SUSPICIOUS_ASSIGNMENT.captures(line) {
                let field = caps.get(1).unwrap().as_str();
                let value = caps.get(2).unwrap().as_str();
                if !looks_like_a_placeholder(value) {
                    findings.push(SecurityFinding {
                        file: file.path.clone(),
                        line: line_no,
                        kind: SecurityFindingKind::SuspiciousAssignment,
                        severity: Severity::Medium,
                        message: format!(
                            "a literal value assigned to a credential-shaped field ({field})"
                        ),
                    });
                }
            }
        }
    }

    // Deterministic order: worst severity first, then file, then line --
    // a stable order so this report doesn't reshuffle between otherwise-
    // identical runs.
    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{FileRecord, Language};

    fn index_with_file(dir: &std::path::Path, name: &str, content: &str) -> RepoIndex {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        RepoIndex {
            root: dir.to_path_buf(),
            files: vec![FileRecord {
                path,
                language: Language::Rust,
                lines: content.lines().count(),
                symbols: vec![],
                imports: vec![],
                calls: vec![],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        }
    }

    #[test]
    fn detects_an_aws_access_key_id() {
        let dir = tempfile::tempdir().unwrap();
        let index = index_with_file(
            dir.path(),
            "config.rs",
            "let key = \"AKIAABCDEFGHIJKLMNOP\";\n",
        );

        let findings = scan(&index);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecurityFindingKind::AwsAccessKeyId);
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!(findings[0].line, 1);
        assert!(!findings[0].message.contains("AKIA"));
    }

    #[test]
    fn detects_a_private_key_block() {
        let dir = tempfile::tempdir().unwrap();
        let index = index_with_file(
            dir.path(),
            "id_rsa.txt",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIB...\n-----END RSA PRIVATE KEY-----\n",
        );

        let findings = scan(&index);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecurityFindingKind::PrivateKeyBlock);
    }

    #[test]
    fn detects_a_github_token() {
        let dir = tempfile::tempdir().unwrap();
        let index = index_with_file(
            dir.path(),
            "ci.rs",
            &format!("let t = \"ghp_{}\";\n", "a".repeat(36)),
        );

        let findings = scan(&index);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecurityFindingKind::GitHubToken);
    }

    #[test]
    fn detects_a_suspicious_credential_assignment() {
        let dir = tempfile::tempdir().unwrap();
        let index = index_with_file(
            dir.path(),
            "config.py",
            "api_key = \"sk_live_9f8a7b6c5d4e3f2a1b0c\"\n",
        );

        let findings = scan(&index);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecurityFindingKind::SuspiciousAssignment);
        assert_eq!(findings[0].severity, Severity::Medium);
    }

    #[test]
    fn does_not_flag_placeholder_values() {
        let dir = tempfile::tempdir().unwrap();
        let index = index_with_file(
            dir.path(),
            "config.example.py",
            "api_key = \"your-api-key-here-xxxxxxxx\"\n\
             password = \"changeme_please_1234\"\n",
        );

        let findings = scan(&index);

        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn does_not_flag_a_short_or_empty_value() {
        let dir = tempfile::tempdir().unwrap();
        let index = index_with_file(
            dir.path(),
            "config.rs",
            "password = \"\"\npassword = \"short\"\n",
        );

        let findings = scan(&index);

        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_clean_file_produces_no_findings() {
        let dir = tempfile::tempdir().unwrap();
        let index = index_with_file(
            dir.path(),
            "lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        );

        assert!(scan(&index).is_empty());
    }

    #[test]
    fn findings_are_sorted_by_severity_then_file_then_line() {
        let dir = tempfile::tempdir().unwrap();
        let content = format!(
            "api_key = \"sk_live_9f8a7b6c5d4e3f2a1b0c\"\nlet k = \"AKIAABCDEFGHIJKLMNOP\";\nlet t = \"ghp_{}\";\n",
            "a".repeat(36)
        );
        let index = index_with_file(dir.path(), "mixed.rs", &content);

        let findings = scan(&index);

        assert_eq!(findings.len(), 3);
        // High-severity findings (AWS key, GitHub token) sort before the
        // medium-severity suspicious assignment, regardless of line order.
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!(findings[1].severity, Severity::High);
        assert_eq!(findings[2].severity, Severity::Medium);
    }

    #[test]
    fn a_deleted_file_still_in_the_index_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let index = RepoIndex {
            root: dir.path().to_path_buf(),
            files: vec![FileRecord {
                path: dir.path().join("gone.rs"),
                language: Language::Rust,
                lines: 0,
                symbols: vec![],
                imports: vec![],
                calls: vec![],
                field_accesses: vec![],
            }],
            other_files: 0,
            indexed_commit: None,
        };

        assert!(scan(&index).is_empty());
    }
}
