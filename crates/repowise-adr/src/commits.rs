use crate::{DecisionRecord, DecisionSource};
use repowise_git::CommitInfo;

/// Commit messages containing one of these (case-insensitive) are
/// treated as recording a decision. A heuristic, not ground truth: a
/// real decision described without one of these words won't be picked
/// up, and an unrelated commit that happens to mention one will be.
///
/// Widened from the original 7 (`decide` through `instead of`) toward
/// the reference's documented ~19-verb "git archaeology" set (see issue
/// #50): `migrate`/`replace`/`deprecate`/`drop`/`rewrite`/`split`/
/// `revert` are named explicitly in that issue; `opt for`/`in favor
/// of`/`settle on`/`consolidate`/`standardize on` round the list out to
/// 19 from common decision-language vocabulary, since the reference
/// repo wasn't reachable from this session to confirm its exact
/// remaining entries.
const DECISION_KEYWORDS: &[&str] = &[
    "decide",
    "decision",
    "chose",
    "chosen",
    "switch to",
    "adopt",
    "instead of",
    "migrate",
    "replace",
    "deprecate",
    "drop",
    "rewrite",
    "split",
    "revert",
    "opt for",
    "in favor of",
    "settle on",
    "consolidate",
    "standardize on",
];

/// Mine decision-like commits (by message keyword) into `DecisionRecord`s.
pub fn mine_commit_decisions(commits: &[CommitInfo]) -> Vec<DecisionRecord> {
    commits
        .iter()
        .filter(|c| is_decision_message(&c.message))
        .map(|c| DecisionRecord {
            id: format!("commit:{}", short_hash(&c.hash)),
            title: c.message.clone(),
            source: DecisionSource::CommitMessage {
                hash: c.hash.clone(),
                author: c.author.clone(),
            },
            status: None,
            superseded_by: None,
            date: None,
            body: c.message.clone(),
            linked_files: Vec::new(),
        })
        .collect()
}

pub(crate) fn is_decision_message(message: &str) -> bool {
    let lower = message.to_lowercase();
    DECISION_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(7).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(message: &str) -> CommitInfo {
        CommitInfo {
            hash: "abcdef1234567890".to_string(),
            author: "A. Uthor".to_string(),
            message: message.to_string(),
            timestamp: 0,
            files: Vec::new(),
        }
    }

    #[test]
    fn flags_decision_like_messages() {
        let commits = vec![
            commit("Decide to adopt sled over rocksdb for the index store"),
            commit("Fix off-by-one in pagination"),
        ];
        let records = mine_commit_decisions(&commits);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "commit:abcdef1");
        assert!(matches!(
            &records[0].source,
            DecisionSource::CommitMessage { author, .. } if author == "A. Uthor"
        ));
    }

    #[test]
    fn flags_messages_using_the_newly_widened_keyword_set() {
        let decision_like = [
            "Migrate the queue backend to sled",
            "Replace the legacy config loader",
            "Deprecate the old v1 API",
            "Drop support for the ancient client protocol",
            "Rewrite the parser from scratch",
            "Split the monolithic service into two crates",
            "Revert to the previous retry strategy",
            "Opt for a simpler polling loop over webhooks",
            "In favor of composition over inheritance here",
            "Settle on sled as the index store",
            "Consolidate the two config-loading paths",
            "Standardize on snake_case for module names",
        ];
        for message in decision_like {
            assert!(
                is_decision_message(message),
                "expected {message:?} to be flagged as decision-like"
            );
        }
    }

    #[test]
    fn does_not_flag_an_unrelated_message() {
        assert!(!is_decision_message("Fix off-by-one in pagination"));
    }
}
