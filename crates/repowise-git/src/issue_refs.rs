//! GitHub-issue-reference bug-fix detection: a stronger, complementary
//! signal to the message-keyword heuristic in `lib.rs`. A commit
//! message mentioning `#123` where issue 123 is closed with a bug-like
//! label is a much more reliable bug-fix indicator than a keyword like
//! "fix" appearing anywhere in the subject line -- but it needs the
//! GitHub API, so (mirroring `repowise-adr`'s PR-body decision source)
//! it's opt-in behind a `REPOWISE_GITHUB_TOKEN` environment variable and
//! degrades to "no linked issues" rather than failing when there's no
//! token, no GitHub-hosted remote, or a lookup fails.

use serde::Deserialize;

pub const GITHUB_API_BASE: &str = "https://api.github.com";

/// A label containing one of these (case-insensitive) marks an issue as
/// a bug for this heuristic -- the same "substring, not exact match"
/// looseness `BUGFIX_KEYWORDS` already uses for commit messages.
const BUG_LABEL_KEYWORDS: &[&str] = &["bug"];

#[derive(Deserialize)]
struct RawIssue {
    state: String,
    labels: Vec<RawLabel>,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
}

/// `#123`-style issue references in a commit message: a `#` not itself
/// preceded by a word character (so an identifier like `a#1` doesn't
/// match), immediately followed by one or more digits, with no further
/// word character right after (so `#123abc` -- not a plausible issue
/// number -- doesn't match a truncated `123`).
pub fn parse_issue_refs(message: &str) -> Vec<u64> {
    let chars: Vec<char> = message.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' {
            let prev_is_word = i > 0 && chars[i - 1].is_alphanumeric();
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            let has_digits = j > i + 1;
            let next_is_word = j < chars.len() && chars[j].is_alphanumeric();
            if !prev_is_word && has_digits && !next_is_word {
                if let Ok(n) = chars[i + 1..j].iter().collect::<String>().parse() {
                    out.push(n);
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// Whether GitHub issue `number` in `owner/repo` is closed and carries a
/// bug-like label -- `None` if the fetch or parse fails for any reason
/// (network error, rate limit, issue doesn't exist), treated by the
/// caller as "unknown, don't count" rather than a hard failure.
pub fn is_closed_bug_issue(
    base_url: &str,
    owner: &str,
    repo: &str,
    number: u64,
    token: &str,
) -> Option<bool> {
    let url = format!("{base_url}/repos/{owner}/{repo}/issues/{number}");
    let body = ureq::get(&url)
        .set("User-Agent", "rusty_repo_wise-repowise-git")
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let issue: RawIssue = serde_json::from_str(&body).ok()?;
    let is_bug = issue.labels.iter().any(|l| {
        let lower = l.name.to_lowercase();
        BUG_LABEL_KEYWORDS.iter().any(|kw| lower.contains(kw))
    });
    Some(issue.state == "closed" && is_bug)
}

/// Parse `owner/repo` out of a git remote URL. A near-duplicate of
/// `repowise-adr`'s own `parse_github_owner_repo` -- not shared via a
/// cross-crate dependency since `repowise-adr` already depends on
/// `repowise-git`, not the other way around, and this is a small enough
/// helper that duplicating it beats introducing a dependency edge just
/// to share one function.
pub fn parse_github_owner_repo(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();
    let path = trimmed
        .strip_prefix("git@github.com:")
        .or_else(|| trimmed.strip_prefix("ssh://git@github.com/"))
        .or_else(|| trimmed.strip_prefix("https://github.com/"))
        .or_else(|| trimmed.strip_prefix("http://github.com/"))?;
    let path = path.trim_end_matches(".git").trim_end_matches('/');
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hash_style_issue_refs_but_not_markdown_headers_or_hex_colors() {
        assert_eq!(parse_issue_refs("fix #123: null deref"), vec![123]);
        assert_eq!(
            parse_issue_refs("closes #45 and references #67"),
            vec![45, 67]
        );
        // Markdown header, not an issue reference (no digits at all).
        assert_eq!(parse_issue_refs("# Title"), Vec::<u64>::new());
        // A hex color / truncated-looking token -- digits followed by
        // more word characters, so not a plausible bare issue number.
        assert_eq!(parse_issue_refs("background: #123abc"), Vec::<u64>::new());
        // `#` glued onto a preceding identifier isn't a reference either.
        assert_eq!(parse_issue_refs("a#123"), Vec::<u64>::new());
    }

    #[test]
    fn parses_ssh_and_https_github_remotes() {
        assert_eq!(
            parse_github_owner_repo("git@github.com:baileyrd/rusty_repo_wise.git"),
            Some(("baileyrd".to_string(), "rusty_repo_wise".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/baileyrd/rusty_repo_wise"),
            Some(("baileyrd".to_string(), "rusty_repo_wise".to_string()))
        );
    }

    #[test]
    fn rejects_non_github_remotes() {
        assert_eq!(
            parse_github_owner_repo("git@gitlab.com:baileyrd/rusty_repo_wise.git"),
            None
        );
        assert_eq!(parse_github_owner_repo("not a url"), None);
    }

    /// Same hand-rolled fixture-server approach `repowise-adr`'s
    /// `pull_requests` tests use: exercises the real request/response/
    /// JSON-parsing path with no live network call and no mocking crate.
    struct FixtureServer {
        addr: std::net::SocketAddr,
    }

    impl FixtureServer {
        fn start(responses: Vec<&'static str>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                for body in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    use std::io::{Read, Write};
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            FixtureServer { addr }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }
    }

    #[test]
    fn is_closed_bug_issue_reads_state_and_labels_from_the_real_json_path() {
        let bug_response = r#"{"state": "closed", "labels": [{"name": "bug"}, {"name": "P1"}]}"#;
        let feature_response = r#"{"state": "closed", "labels": [{"name": "enhancement"}]}"#;
        let open_bug_response = r#"{"state": "open", "labels": [{"name": "bug"}]}"#;

        let server = FixtureServer::start(vec![bug_response, feature_response, open_bug_response]);
        let base = server.base_url();

        assert_eq!(
            is_closed_bug_issue(&base, "owner", "repo", 1, "tok"),
            Some(true)
        );
        assert_eq!(
            is_closed_bug_issue(&base, "owner", "repo", 2, "tok"),
            Some(false)
        );
        assert_eq!(
            is_closed_bug_issue(&base, "owner", "repo", 3, "tok"),
            Some(false)
        );
    }
}
