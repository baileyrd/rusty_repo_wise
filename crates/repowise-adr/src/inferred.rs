//! The one decision source that isn't mined from a written artifact.
//!
//! Every other source in this crate reads something a person wrote down
//! on purpose: an ADR file, a commit message, a `WHY:` marker. This one
//! reads decisions a *model* inferred from code while `repowise
//! generate` was writing wiki summaries. That difference is not a
//! footnote — "we chose X because Y" gets read as intent, and a reader
//! needs to know whether the intent was stated or guessed.
//!
//! # Inference happens at write time; mining stays deterministic
//!
//! `repowise-llm` proposes decisions during `generate` and writes them
//! here. This module only *reads* that file. So `repowise decisions`,
//! `get_why`, and the dashboard stay exactly as deterministic and
//! offline as they were — no LLM call happens on a read path, and the
//! same repo state produces the same answer every time.
//!
//! # Anchors are text, not line numbers
//!
//! Each proposal carries a verbatim snippet from the file it's about,
//! and is only kept if that snippet **still literally appears there**.
//! Two things follow, and both matter:
//!
//! - A model that invented a plausible-sounding justification for code
//!   that doesn't exist gets dropped at the door, without anyone having
//!   to notice.
//! - A decision about code that has since been deleted or rewritten
//!   drops itself on the next read, rather than lingering as confident
//!   commentary on a file that no longer says that.
//!
//! A line number would do neither: it stays valid-looking while the line
//! under it changes. The current line is recomputed from where the
//! anchor actually is, so it's right by construction rather than by
//! being refreshed.

use crate::{DecisionRecord, DecisionSource};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where `repowise generate` writes proposals, relative to the index dir.
pub const INFERRED_FILE: &str = "inferred-decisions.json";

pub fn store_path(root: &Path) -> PathBuf {
    root.join(repowise_core::RepoIndex::INDEX_DIR)
        .join(INFERRED_FILE)
}

/// One decision a model inferred, as written by `repowise generate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferredDecision {
    pub title: String,
    /// Why the model thinks this decision was made. Becomes the record's
    /// body, so it's what linking and display read.
    pub rationale: String,
    /// Repo-relative path of the file the anchor is in.
    pub file: String,
    /// Verbatim text from that file.
    ///
    /// Deliberately not a line number — see the module doc. This is the
    /// entire basis on which a proposal is trusted, so it's stored as
    /// written and checked as written.
    pub anchor: String,
}

/// The proposals on disk, plus which model produced them.
///
/// The model is recorded because a reader judging an inferred claim is
/// entitled to know what inferred it, and because two models' output
/// shouldn't silently merge into one undifferentiated pile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferredStore {
    pub model: String,
    pub decisions: Vec<InferredDecision>,
}

impl InferredStore {
    pub fn load(root: &Path) -> Option<Self> {
        let raw = std::fs::read_to_string(store_path(root)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn save(&self, root: &Path) -> anyhow::Result<PathBuf> {
        let path = store_path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(path)
    }
}

/// What this source contributed, and why, if it contributed nothing.
///
/// An empty decision list is ambiguous in the worst direction: "this
/// repo has no inferred decisions" and "you never ran the pass that
/// produces them" look identical, and only one of them is a fact about
/// the repo. Every surface that displays decisions reports this instead
/// of leaving the reader to guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferredState {
    /// No store on disk — `repowise generate` hasn't run with an LLM
    /// endpoint configured. This is the default state and not a problem.
    NotGenerated,
    /// A store exists.
    Loaded {
        model: String,
        kept: usize,
        /// Proposals dropped because their anchor text no longer appears
        /// in the file. Reported rather than silently swallowed: a
        /// non-zero count means the code moved out from under a claim,
        /// which is worth knowing.
        dropped_stale: usize,
    },
}

impl InferredState {
    /// A one-line description for display. Always says something —
    /// silence is what this type exists to prevent.
    pub fn describe(&self) -> String {
        match self {
            InferredState::NotGenerated => format!(
                "LLM-inferred decisions: none stored. This source is opt-in -- run \
                 `repowise generate` with {} set to populate it. Every other decision \
                 source is unaffected.",
                repowise_llm_base_url_var()
            ),
            InferredState::Loaded {
                model,
                kept,
                dropped_stale,
            } => {
                let mut s = format!(
                    "LLM-inferred decisions: {kept} from model {model:?}. These are a \
                     model's reading of the code, not anything a person wrote down."
                );
                if *dropped_stale == 1 {
                    s.push_str(
                        " 1 more was dropped because the code it quoted is no longer in \
                         the file.",
                    );
                } else if *dropped_stale > 1 {
                    s.push_str(&format!(
                        " {dropped_stale} more were dropped because the code they quoted \
                         is no longer in the file."
                    ));
                }
                s
            }
        }
    }
}

/// Spelled here rather than depending on `repowise-llm` for one string:
/// this crate is the deterministic half and doesn't take that dependency
/// in the other direction. `repowise-llm` owns the constant; a test in
/// this crate's suite would only re-assert what that crate already
/// tests, so this is a doc-level duplication, not a functional one.
fn repowise_llm_base_url_var() -> &'static str {
    "REPOWISE_LLM_BASE_URL"
}

/// Read the store and turn each surviving proposal into a `DecisionRecord`.
///
/// A proposal survives only if its anchor text is still found verbatim in
/// its file. Everything else — a file that's gone, an anchor that was
/// never there, code that has since been rewritten — is dropped rather
/// than downgraded, because a decision record with a broken citation is
/// exactly the thing that gets read as intent while being unsupported.
pub fn mine_inferred_decisions(root: &Path) -> (Vec<DecisionRecord>, InferredState) {
    let Some(store) = InferredStore::load(root) else {
        return (Vec::new(), InferredState::NotGenerated);
    };

    let mut records = Vec::new();
    let mut dropped_stale = 0usize;

    for decision in &store.decisions {
        let path = root.join(&decision.file);
        let Ok(contents) = std::fs::read_to_string(&path) else {
            dropped_stale += 1;
            continue;
        };
        let Some(line) = anchor_line(&contents, &decision.anchor) else {
            dropped_stale += 1;
            continue;
        };

        records.push(DecisionRecord {
            id: format!("inferred:{}:{line}", decision.file),
            title: decision.title.clone(),
            source: DecisionSource::Inferred {
                file: path.clone(),
                line,
                model: store.model.clone(),
            },
            status: None,
            superseded_by: None,
            date: None,
            body: decision.rationale.clone(),
            // Authoritative, like a code comment's own file: the anchor
            // was found in this file, so there's nothing for text
            // matching to improve and plenty for it to get wrong.
            linked_files: vec![path],
        });
    }

    let state = InferredState::Loaded {
        model: store.model.clone(),
        kept: records.len(),
        dropped_stale,
    };
    (records, state)
}

/// 1-based line where `anchor` starts in `contents`, or `None` if it
/// isn't there.
///
/// Whitespace is normalized on both sides before comparing, because a
/// model reproducing a line of code reliably gets the code right and
/// unreliably gets the indentation right — and indentation is not what
/// the check is for. Nothing else is normalized: the identifiers,
/// operators and literals have to match exactly, which is the part that
/// distinguishes a quote from an invention.
pub fn anchor_line(contents: &str, anchor: &str) -> Option<usize> {
    let needle = normalize(anchor);
    if needle.is_empty() {
        return None;
    }

    let lines: Vec<&str> = contents.lines().collect();
    let anchor_lines: Vec<String> = anchor
        .lines()
        .map(normalize)
        .filter(|l| !l.is_empty())
        .collect();
    if anchor_lines.is_empty() {
        return None;
    }

    // Single-line anchors are the common case and can match a substring
    // of a line; multi-line anchors must match consecutive lines.
    if anchor_lines.len() == 1 {
        return lines
            .iter()
            .position(|l| normalize(l).contains(&anchor_lines[0]))
            .map(|i| i + 1);
    }

    let normalized: Vec<String> = lines.iter().map(|l| normalize(l)).collect();
    let non_empty: Vec<(usize, &String)> = normalized
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.is_empty())
        .collect();

    non_empty
        .windows(anchor_lines.len())
        .find(|window| {
            window
                .iter()
                .zip(&anchor_lines)
                .all(|((_, actual), wanted)| actual.contains(wanted.as_str()))
        })
        .map(|window| window[0].0 + 1)
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(root: &Path, decisions: Vec<InferredDecision>) {
        InferredStore {
            model: "test-model".to_string(),
            decisions,
        }
        .save(root)
        .unwrap();
    }

    fn decision(file: &str, anchor: &str) -> InferredDecision {
        InferredDecision {
            title: "Chose a bounded queue".to_string(),
            rationale: "The queue is bounded, so backpressure is preferred to unbounded \
                        memory growth."
                .to_string(),
            file: file.to_string(),
            anchor: anchor.to_string(),
        }
    }

    #[test]
    fn no_store_is_a_named_state_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let (records, state) = mine_inferred_decisions(&root);
        assert!(records.is_empty());
        assert_eq!(state, InferredState::NotGenerated);
        // The distinction this type exists for: the reader must be able
        // to tell "no decisions here" from "you never ran the pass".
        assert!(state.describe().contains("opt-in"));
        assert!(state.describe().contains("REPOWISE_LLM_BASE_URL"));
    }

    #[test]
    fn a_decision_whose_anchor_is_present_survives_and_gets_the_real_line() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("q.rs"),
            "// header\n\nfn main() {}\n\nlet queue = Bounded::new(128);\n",
        )
        .unwrap();
        store(
            &root,
            vec![decision("q.rs", "let queue = Bounded::new(128);")],
        );

        let (records, state) = mine_inferred_decisions(&root);
        assert_eq!(records.len(), 1);
        let DecisionSource::Inferred { line, model, .. } = &records[0].source else {
            panic!("wrong source: {:?}", records[0].source);
        };
        // Line 5, computed from where the anchor actually is -- the
        // store never held a line number to be wrong about.
        assert_eq!(*line, 5);
        assert_eq!(model, "test-model");
        assert_eq!(
            state,
            InferredState::Loaded {
                model: "test-model".to_string(),
                kept: 1,
                dropped_stale: 0,
            }
        );
    }

    /// The hallucination filter. A model that invents a justification
    /// for code that isn't there must produce nothing, not a plausible
    /// decision record.
    #[test]
    fn a_decision_whose_anchor_is_absent_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("q.rs"), "fn main() {}\n").unwrap();
        store(
            &root,
            vec![decision("q.rs", "let queue = Bounded::new(128);")],
        );

        let (records, state) = mine_inferred_decisions(&root);
        assert!(records.is_empty(), "{records:?}");
        assert_eq!(
            state,
            InferredState::Loaded {
                model: "test-model".to_string(),
                kept: 0,
                dropped_stale: 1,
            }
        );
        assert!(state.describe().contains("no longer in the file"));
    }

    /// Staleness handles itself: the anchor is re-checked on every read,
    /// so a rewritten file drops its own commentary without anyone
    /// running an invalidation step.
    #[test]
    fn rewriting_the_anchored_code_drops_the_decision_on_the_next_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("q.rs");
        std::fs::write(&file, "let queue = Bounded::new(128);\n").unwrap();
        store(
            &root,
            vec![decision("q.rs", "let queue = Bounded::new(128);")],
        );
        assert_eq!(mine_inferred_decisions(&root).0.len(), 1);

        std::fs::write(&file, "let queue = Unbounded::new();\n").unwrap();
        let (records, _) = mine_inferred_decisions(&root);
        assert!(
            records.is_empty(),
            "a decision quoting code that was replaced must not survive: {records:?}"
        );
    }

    #[test]
    fn a_deleted_file_drops_its_decisions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        store(&root, vec![decision("gone.rs", "anything")]);

        let (records, state) = mine_inferred_decisions(&root);
        assert!(records.is_empty());
        assert!(matches!(
            state,
            InferredState::Loaded {
                dropped_stale: 1,
                ..
            }
        ));
    }

    #[test]
    fn indentation_differences_do_not_break_an_otherwise_exact_quote() {
        let contents = "fn f() {\n        let queue = Bounded::new(128);\n}\n";
        assert_eq!(
            anchor_line(contents, "let queue = Bounded::new(128);"),
            Some(2)
        );
        assert_eq!(
            anchor_line(contents, "let  queue  =  Bounded::new(128);"),
            Some(2)
        );
    }

    /// Whitespace is forgiven; the code itself is not.
    #[test]
    fn a_changed_literal_is_not_an_anchor_match() {
        let contents = "let queue = Bounded::new(128);\n";
        assert_eq!(
            anchor_line(contents, "let queue = Bounded::new(256);"),
            None
        );
        assert_eq!(
            anchor_line(contents, "let queue = Unbounded::new(128);"),
            None
        );
    }

    #[test]
    fn a_multi_line_anchor_must_match_consecutive_lines() {
        let contents = "fn a() {}\n\nfn b() {\n    retry(3);\n}\n\nfn c() {}\n";
        assert_eq!(anchor_line(contents, "fn b() {\n    retry(3);\n}"), Some(3));
        // Same lines, wrong order -- not a quote of this file.
        assert_eq!(anchor_line(contents, "    retry(3);\nfn b() {"), None);
    }

    #[test]
    fn an_empty_anchor_never_matches() {
        assert_eq!(anchor_line("anything at all\n", ""), None);
        assert_eq!(anchor_line("anything at all\n", "   \n  "), None);
    }
}
