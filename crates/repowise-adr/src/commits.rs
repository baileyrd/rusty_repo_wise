use crate::{DecisionRecord, DecisionSource};
use repowise_git::CommitInfo;

/// Commit messages containing one of these (case-insensitive) are
/// treated as recording a decision. A heuristic, not ground truth: a
/// real decision described without one of these words won't be picked
/// up, and an unrelated commit that happens to mention one will be.
const DECISION_KEYWORDS: &[&str] = &[
    "decide",
    "decision",
    "chose",
    "chosen",
    "switch to",
    "adopt",
    "instead of",
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

fn is_decision_message(message: &str) -> bool {
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
}
