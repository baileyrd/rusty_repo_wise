//! Asking a model what architectural decisions a file's code implies,
//! during the `repowise generate` pass that's already reading it.
//!
//! This is the write half of the LLM-inferred decision source;
//! `repowise_adr::inferred` is the read half. The split is the point:
//! inference happens once, here, behind an opt-in flag, and every read
//! path (`repowise decisions`, `get_why`, the dashboard) stays offline
//! and deterministic.
//!
//! # Precision over recall, enforced rather than requested
//!
//! A wrong decision record is worse than a missing one, because it gets
//! read as intent. Asking a model nicely for precision is not a control,
//! so the pass doesn't rely on it:
//!
//! - Every proposal must quote **verbatim text from the file**. The
//!   quote is checked against the file before the proposal is stored,
//!   and again on every read. A model that invents a justification for
//!   code that doesn't exist produces nothing.
//! - Proposals missing a title, rationale, or anchor are dropped, not
//!   patched up with a placeholder.
//! - Malformed model output drops that file's proposals and nothing
//!   else. One file's bad JSON must not take the pass down.
//!
//! Every one of those is a *drop*. There is no branch in this module
//! that turns a doubtful proposal into a stored one.

use crate::LlmConfig;
use repowise_adr::inferred::{anchor_line, InferredDecision, InferredStore};
use repowise_core::{FileRecord, RepoIndex};
use serde::Deserialize;
use std::path::Path;

const PROPOSAL_SYSTEM_PROMPT: &str = "You identify architectural decisions that a source file's \
code demonstrates -- deliberate technical choices with a rationale a reader would benefit from \
knowing, such as a chosen concurrency strategy, an error-handling convention, a deliberate \
constraint, or a tradeoff the code encodes.

Reply with a JSON array and nothing else. Each element must be an object with exactly these keys:
  \"title\":     a short noun phrase naming the decision.
  \"rationale\": one or two sentences on what was decided and why the code suggests it.
  \"anchor\":    a VERBATIM span copied character-for-character from the file, 1-3 lines long, \
that demonstrates the decision.

The anchor is checked against the file. Any element whose anchor does not literally appear there \
is discarded, so do not paraphrase, summarize, reconstruct from memory, or quote a line you are \
not certain of.

Report only decisions the code actually evidences. A file that shows no notable decision must \
get an empty array -- that is a correct and expected answer, not a failure. Prefer returning \
nothing over returning something plausible.";

/// One file's outcome in the proposal pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileProposalResult {
    /// Proposals the model returned that survived every check.
    pub kept: usize,
    /// Returned but discarded because their anchor wasn't in the file.
    /// The hallucination count, and worth reporting as its own number:
    /// a pass where this dominates says something about the model that
    /// a total would hide.
    pub dropped_unanchored: usize,
    /// Discarded for a missing title or rationale.
    pub dropped_incomplete: usize,
    /// The model's reply couldn't be parsed as the requested JSON, so
    /// this file contributed nothing.
    pub unparseable: bool,
    /// The LLM call itself failed.
    pub call_failed: bool,
}

/// Totals across the pass, reported to the user by `repowise generate`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProposalReport {
    pub files_considered: usize,
    pub kept: usize,
    pub dropped_unanchored: usize,
    pub dropped_incomplete: usize,
    pub unparseable_files: usize,
    pub failed_files: usize,
}

/// What the model is asked to return, per proposal.
#[derive(Debug, Deserialize)]
struct RawProposal {
    #[serde(default)]
    title: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    anchor: String,
}

/// Propose decisions for every indexed file and write the survivors to
/// the store `repowise-adr` reads.
///
/// Replaces the store wholesale rather than merging: a decision the
/// model no longer proposes for code that still exists should disappear,
/// and a merge would preserve it forever. Re-running is therefore
/// idempotent in the way a reader expects — the store reflects this run,
/// not the union of every run.
pub fn propose_decisions(
    index: &RepoIndex,
    config: &LlmConfig,
) -> anyhow::Result<(ProposalReport, Vec<InferredDecision>)> {
    let mut report = ProposalReport::default();
    let mut decisions: Vec<InferredDecision> = Vec::new();

    for file in &index.files {
        report.files_considered += 1;
        let Ok(contents) = std::fs::read_to_string(&file.path) else {
            // Indexed but unreadable now. Not a model failure, and not
            // something to count against it.
            report.files_considered -= 1;
            continue;
        };
        let rel = relative(&index.root, file);

        let (result, kept) = propose_for_file(config, &rel, &contents);
        report.kept += result.kept;
        report.dropped_unanchored += result.dropped_unanchored;
        report.dropped_incomplete += result.dropped_incomplete;
        report.unparseable_files += usize::from(result.unparseable);
        report.failed_files += usize::from(result.call_failed);
        decisions.extend(kept);
    }

    InferredStore {
        model: config.model.clone(),
        decisions: decisions.clone(),
    }
    .save(&index.root)?;

    Ok((report, decisions))
}

/// One file's round trip, with every check applied.
///
/// Split out from [`propose_decisions`] so the filtering is testable
/// without a network call: [`filter_proposals`] is the part that
/// decides, and it's a pure function.
fn propose_for_file(
    config: &LlmConfig,
    rel: &str,
    contents: &str,
) -> (FileProposalResult, Vec<InferredDecision>) {
    let user = format!("File: {rel}\n\n{contents}");
    let reply = match crate::complete(config, PROPOSAL_SYSTEM_PROMPT, &user) {
        Ok(reply) => reply,
        Err(_) => {
            return (
                FileProposalResult {
                    kept: 0,
                    dropped_unanchored: 0,
                    dropped_incomplete: 0,
                    unparseable: false,
                    call_failed: true,
                },
                Vec::new(),
            )
        }
    };
    filter_proposals(rel, contents, &reply)
}

/// Parse a model reply and keep only the proposals that survive every
/// check. Pure: no network, no filesystem.
pub fn filter_proposals(
    rel: &str,
    contents: &str,
    reply: &str,
) -> (FileProposalResult, Vec<InferredDecision>) {
    let mut result = FileProposalResult {
        kept: 0,
        dropped_unanchored: 0,
        dropped_incomplete: 0,
        unparseable: false,
        call_failed: false,
    };

    let Some(raw) = parse_proposals(reply) else {
        result.unparseable = true;
        return (result, Vec::new());
    };

    let mut kept = Vec::new();
    for proposal in raw {
        let title = proposal.title.trim().to_string();
        let rationale = proposal.rationale.trim().to_string();
        let anchor = proposal.anchor.trim().to_string();

        if title.is_empty() || rationale.is_empty() || anchor.is_empty() {
            result.dropped_incomplete += 1;
            continue;
        }
        // The check that makes this source usable. Note it runs against
        // the file's real contents, not against anything the model said
        // about them.
        if anchor_line(contents, &anchor).is_none() {
            result.dropped_unanchored += 1;
            continue;
        }
        // Two proposals anchored to the same line are one decision
        // stated twice; keeping both inflates the count without adding
        // information.
        if kept.iter().any(|k: &InferredDecision| k.anchor == anchor) {
            result.dropped_incomplete += 1;
            continue;
        }

        result.kept += 1;
        kept.push(InferredDecision {
            title,
            rationale,
            file: rel.to_string(),
            anchor,
        });
    }

    (result, kept)
}

/// Pull a JSON array out of a model reply.
///
/// Tolerant of a fenced code block around it, since that's the single
/// most common way an otherwise-correct reply arrives — but not tolerant
/// of anything that isn't the requested array. Guessing at malformed
/// output is how a proposal nobody vetted ends up stored.
fn parse_proposals(reply: &str) -> Option<Vec<RawProposal>> {
    let trimmed = reply.trim();
    let unfenced = strip_code_fence(trimmed);
    let start = unfenced.find('[')?;
    let end = unfenced.rfind(']')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&unfenced[start..=end]).ok()
}

fn strip_code_fence(s: &str) -> &str {
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    // Drop an optional language tag on the fence's own line.
    let rest = rest.split_once('\n').map(|(_, body)| body).unwrap_or(rest);
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

fn relative(root: &Path, file: &FileRecord) -> String {
    file.path
        .strip_prefix(root)
        .unwrap_or(&file.path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "fn main() {\n    let queue = Bounded::new(128);\n}\n";

    fn reply(anchor: &str) -> String {
        format!(
            r#"[{{"title": "Bounded queue", "rationale": "Backpressure over unbounded growth.",
                  "anchor": {}}}]"#,
            serde_json::to_string(anchor).unwrap()
        )
    }

    #[test]
    fn a_proposal_quoting_the_file_is_kept() {
        let (result, kept) =
            filter_proposals("q.rs", FILE, &reply("let queue = Bounded::new(128);"));
        assert_eq!(result.kept, 1);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].file, "q.rs");
        assert_eq!(kept[0].title, "Bounded queue");
    }

    /// The control this source rests on. A confident, well-formed,
    /// entirely invented proposal must produce nothing.
    #[test]
    fn a_proposal_quoting_code_that_is_not_there_is_dropped() {
        let (result, kept) =
            filter_proposals("q.rs", FILE, &reply("let queue = Unbounded::new();"));
        assert_eq!(result.dropped_unanchored, 1);
        assert_eq!(result.kept, 0);
        assert!(kept.is_empty());
    }

    #[test]
    fn a_proposal_missing_a_field_is_dropped_not_patched() {
        let json = r#"[{"title": "", "rationale": "Something", "anchor": "let queue = Bounded::new(128);"},
                       {"title": "T", "rationale": "", "anchor": "let queue = Bounded::new(128);"},
                       {"title": "T", "rationale": "R", "anchor": ""}]"#;
        let (result, kept) = filter_proposals("q.rs", FILE, json);
        assert_eq!(result.dropped_incomplete, 3);
        assert!(kept.is_empty());
    }

    #[test]
    fn an_empty_array_is_a_valid_answer_not_a_parse_failure() {
        let (result, kept) = filter_proposals("q.rs", FILE, "[]");
        assert!(!result.unparseable, "an empty array is the requested shape");
        assert_eq!(result.kept, 0);
        assert!(kept.is_empty());
    }

    #[test]
    fn a_fenced_array_still_parses() {
        let fenced = format!("```json\n{}\n```", reply("let queue = Bounded::new(128);"));
        let (result, kept) = filter_proposals("q.rs", FILE, &fenced);
        assert!(!result.unparseable);
        assert_eq!(kept.len(), 1);
    }

    /// Prose where JSON was asked for must contribute nothing, rather
    /// than being mined for something that looks like a decision.
    #[test]
    fn unparseable_output_contributes_nothing() {
        for reply in [
            "I think this file decides to use a bounded queue.",
            "{\"title\": \"not an array\"}",
            "",
        ] {
            let (result, kept) = filter_proposals("q.rs", FILE, reply);
            assert!(result.unparseable, "should be unparseable: {reply:?}");
            assert!(kept.is_empty());
        }
    }

    #[test]
    fn two_proposals_on_the_same_anchor_collapse_to_one() {
        let json = r#"[{"title": "A", "rationale": "R", "anchor": "let queue = Bounded::new(128);"},
                       {"title": "B", "rationale": "R", "anchor": "let queue = Bounded::new(128);"}]"#;
        let (result, kept) = filter_proposals("q.rs", FILE, json);
        assert_eq!(result.kept, 1);
        assert_eq!(kept.len(), 1);
    }

    /// The prompt makes a promise the code has to keep, or it's just
    /// words: it tells the model unanchored elements are discarded.
    #[test]
    fn the_prompt_states_the_check_that_is_actually_enforced() {
        assert!(PROPOSAL_SYSTEM_PROMPT.contains("VERBATIM"));
        assert!(PROPOSAL_SYSTEM_PROMPT.contains("discarded"));
        assert!(
            PROPOSAL_SYSTEM_PROMPT.contains("empty array"),
            "the model must be told that finding nothing is a correct answer"
        );
    }
}
