//! Merged-PR-body decision mining: the one decision source that needs
//! the GitHub API rather than local git/filesystem data, and the first
//! network dependency `repowise-adr` (previously pure git/filesystem)
//! has taken on.
//!
//! Deliberately conservative about when it makes a network call at all:
//! `mine_pull_requests` (in `lib.rs`) only attempts one when a
//! `REPOWISE_GITHUB_TOKEN` environment variable is set. A local
//! codebase-analysis CLI making unsolicited outbound HTTP requests would
//! be surprising; requiring an explicit opt-in token — rather than
//! falling back to GitHub's unauthenticated (and much more rate-limited)
//! API — keeps that behavior opt-in rather than a surprise. No token, no
//! remote, or a remote that isn't GitHub all degrade to an empty result,
//! the same "not required" tradeoff already used for git history/ADR
//! files elsewhere in this crate.

use crate::commits::is_decision_message;
use crate::{DecisionRecord, DecisionSource};
use serde::Deserialize;
use std::path::Path;

pub const GITHUB_API_BASE: &str = "https://api.github.com";

/// The minimal shape of a merged pull request needed for decision
/// mining: enough to apply the same keyword heuristic `commits.rs` uses,
/// and the file list the GitHub API already reports (so linking doesn't
/// need to fall back to text-matching, unlike ADR files/commit messages).
#[derive(Debug, Clone)]
pub struct MergedPullRequest {
    pub number: u64,
    pub author: String,
    pub title: String,
    pub body: String,
    /// Repo-relative paths, as reported by the GitHub API.
    pub files: Vec<String>,
}

#[derive(Deserialize)]
struct RawPull {
    number: u64,
    title: String,
    body: Option<String>,
    user: RawUser,
    merged_at: Option<String>,
}

#[derive(Deserialize)]
struct RawUser {
    login: String,
}

#[derive(Deserialize)]
struct RawFile {
    filename: String,
}

/// Fetch merged PRs for `owner/repo` from a GitHub-API-compatible
/// endpoint at `base_url`. Production callers pass `GITHUB_API_BASE`;
/// tests point this at a local fixture server instead, so no test here
/// makes a live network call. `token`, if given, is sent as a bearer
/// auth header.
pub fn fetch_merged_pull_requests(
    base_url: &str,
    owner: &str,
    repo: &str,
    token: Option<&str>,
) -> anyhow::Result<Vec<MergedPullRequest>> {
    let list_url = format!("{base_url}/repos/{owner}/{repo}/pulls?state=closed&per_page=50");
    let raw_pulls: Vec<RawPull> = get_json(&list_url, token)?;

    let mut merged = Vec::new();
    for pr in raw_pulls.into_iter().filter(|p| p.merged_at.is_some()) {
        let files_url = format!("{base_url}/repos/{owner}/{repo}/pulls/{}/files", pr.number);
        // A single PR's file list failing to fetch shouldn't fail the
        // whole mine — treat it as "no known files" for that PR.
        let raw_files: Vec<RawFile> = get_json(&files_url, token).unwrap_or_default();
        merged.push(MergedPullRequest {
            number: pr.number,
            author: pr.user.login,
            title: pr.title,
            body: pr.body.unwrap_or_default(),
            files: raw_files.into_iter().map(|f| f.filename).collect(),
        });
    }
    Ok(merged)
}

fn get_json<T: serde::de::DeserializeOwned>(url: &str, token: Option<&str>) -> anyhow::Result<T> {
    let mut req = ureq::get(url).set("User-Agent", "rusty_repo_wise-repowise-adr");
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let body = req.call()?.into_string()?;
    Ok(serde_json::from_str(&body)?)
}

/// Mine decision-like merged PRs (by the same title/body keyword
/// heuristic `commits.rs` uses) into `DecisionRecord`s, linked to the
/// files that PR actually touched per the GitHub API — not
/// text-matching, unlike the ADR-file/commit-message sources.
pub fn mine_pull_request_decisions(prs: &[MergedPullRequest], root: &Path) -> Vec<DecisionRecord> {
    prs.iter()
        .filter(|pr| is_decision_message(&pr.body) || is_decision_message(&pr.title))
        .map(|pr| DecisionRecord {
            id: format!("pr:{}", pr.number),
            title: pr.title.clone(),
            source: DecisionSource::PullRequest {
                number: pr.number,
                author: pr.author.clone(),
            },
            status: None,
            superseded_by: None,
            date: None,
            body: pr.body.clone(),
            linked_files: pr.files.iter().map(|f| root.join(f)).collect(),
        })
        .collect()
}

/// Parse `owner/repo` out of a git remote URL: SSH
/// (`git@github.com:owner/repo.git`), HTTPS
/// (`https://github.com/owner/repo(.git)?`), or `ssh://` forms. `None`
/// for any other host or an unparseable URL — this source only makes
/// sense for a GitHub-hosted remote.
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
    use std::path::PathBuf;

    #[test]
    fn parses_ssh_and_https_github_remotes() {
        assert_eq!(
            parse_github_owner_repo("git@github.com:baileyrd/rusty_repo_wise.git"),
            Some(("baileyrd".to_string(), "rusty_repo_wise".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/baileyrd/rusty_repo_wise.git"),
            Some(("baileyrd".to_string(), "rusty_repo_wise".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/baileyrd/rusty_repo_wise"),
            Some(("baileyrd".to_string(), "rusty_repo_wise".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("ssh://git@github.com/baileyrd/rusty_repo_wise.git"),
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

    #[test]
    fn mines_decision_like_merged_prs_linked_to_their_real_files() {
        let root = PathBuf::from("/repo");
        let prs = vec![
            MergedPullRequest {
                number: 12,
                author: "octocat".to_string(),
                title: "Decide to adopt sled over rocksdb".to_string(),
                body: "We chose sled for the index store.".to_string(),
                files: vec!["src/index.rs".to_string(), "Cargo.toml".to_string()],
            },
            MergedPullRequest {
                number: 13,
                author: "octocat".to_string(),
                title: "Fix a typo".to_string(),
                body: "Just a small wording fix in the README.".to_string(),
                files: vec!["README.md".to_string()],
            },
        ];

        let records = mine_pull_request_decisions(&prs, &root);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "pr:12");
        assert_eq!(
            records[0].linked_files,
            vec![root.join("src/index.rs"), root.join("Cargo.toml")]
        );
        assert!(matches!(
            &records[0].source,
            DecisionSource::PullRequest { number: 12, author } if author == "octocat"
        ));
    }

    /// Runs `fetch_merged_pull_requests` against a hand-rolled, minimal
    /// HTTP/1.1 server bound to a local port — exercises the real
    /// request/response/JSON-parsing path without a live network call
    /// or a mocking crate dependency, the same "real thing, disposable
    /// fixture" approach `repowise-git`'s tests already take with real
    /// git repos.
    struct FixtureServer {
        addr: std::net::SocketAddr,
    }

    impl FixtureServer {
        /// Serves `body` (assumed to be a complete JSON response) for
        /// every request it receives, on a background thread, for
        /// `requests` total requests before shutting down.
        fn start(responses: Vec<&'static str>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                for body in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    // Drain the request so the client's write doesn't
                    // block on a full socket buffer; we don't need to
                    // parse it since this fixture is unconditional.
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
    fn fetch_merged_pull_requests_parses_the_real_http_and_json_path() {
        let list_response = r#"[
            {"number": 1, "title": "Decide to use sled", "body": "chose sled", "user": {"login": "octocat"}, "merged_at": "2026-01-01T00:00:00Z"},
            {"number": 2, "title": "Open, not merged", "body": "", "user": {"login": "octocat"}, "merged_at": null}
        ]"#;
        let files_response = r#"[{"filename": "src/lib.rs"}]"#;

        let server = FixtureServer::start(vec![list_response, files_response]);

        let prs = fetch_merged_pull_requests(&server.base_url(), "owner", "repo", None).unwrap();

        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 1);
        assert_eq!(prs[0].author, "octocat");
        assert_eq!(prs[0].files, vec!["src/lib.rs".to_string()]);
    }
}
