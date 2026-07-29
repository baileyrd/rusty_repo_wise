//! Retrieval for question answering, shared by the dashboard's
//! `POST /api/chat` and the MCP `get_answer` tool.
//!
//! This used to live in `repowise-server`. It moved here so the two
//! surfaces run the same code: an agent asking over stdio and a browser
//! asking over HTTP should not be able to get different answers to the
//! same question because one of them drifted.
//!
//! # Two things this reports that the earlier version didn't
//!
//! **Which files the answer drew on.** The context string always
//! contained them inline, but only as prose for the model to read --
//! nothing structured came back out. An answer whose sources can't be
//! listed can't be checked, and "cite what you used" is most of what
//! makes a generated answer auditable rather than merely fluent.
//!
//! **Which retrieval actually ran.** Embedding retrieval falls back to
//! keyword matching whenever the embeddings call fails -- an endpoint
//! that doesn't implement `/v1/embeddings`, a bad response, a timeout.
//! That fallback is *materially weaker*: substring matching over paths
//! and symbol names finds nothing for a question phrased in concepts.
//! Previously it happened silently, so a caller had no way to tell a
//! semantic answer from a degraded one. Now [`Retrieval::mode`] says.

use crate::LlmConfig;
use repowise_core::{FileRecord, RepoIndex};
use std::path::Path;

/// Files handed to the model as context.
///
/// 10, carried over from the original chat implementation.
pub const CONTEXT_LIMIT: usize = 10;

const PREAMBLE: &str = "You are a helpful assistant answering questions about a codebase. \
     Base your answers only on the information below; if you don't have \
     enough information to answer, say so rather than guessing.\n\n";

/// How the context was assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrievalMode {
    /// Embeddings, ranked by cosine similarity. What this is meant to do.
    Semantic,
    /// Substring matching over file paths and symbol names, used when
    /// the embeddings call failed. Finds nothing for a conceptually
    /// phrased question, so an answer built on it deserves a caveat.
    Keyword,
}

impl RetrievalMode {
    pub fn label(&self) -> &'static str {
        match self {
            RetrievalMode::Semantic => "semantic",
            RetrievalMode::Keyword => "keyword",
        }
    }

    /// Why a caller should care which one ran. `None` for the healthy
    /// case -- a caveat printed unconditionally is one nobody reads.
    pub fn caveat(&self) -> Option<&'static str> {
        match self {
            RetrievalMode::Semantic => None,
            RetrievalMode::Keyword => Some(
                "Embedding retrieval was unavailable, so context came from substring \
                 matching over file paths and symbol names. That finds little for a \
                 question phrased conceptually rather than by name -- treat a thin or \
                 unhelpful answer as a retrieval failure rather than as evidence the \
                 codebase lacks the thing asked about.",
            ),
        }
    }
}

/// Context for the model, plus what it was built from.
#[derive(Debug, Clone)]
pub struct Retrieval {
    /// The system prompt.
    pub context: String,
    /// Repo-relative paths the context drew on, best first.
    pub cited: Vec<String>,
    pub mode: RetrievalMode,
}

impl Retrieval {
    /// Did retrieval find anything at all?
    ///
    /// Distinct from "the answer was unhelpful". An empty citation list
    /// means the question was answered from no sources, which a caller
    /// should be able to detect without parsing prose.
    pub fn found_sources(&self) -> bool {
        !self.cited.is_empty()
    }
}

fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

/// A per-file text unit for embedding.
///
/// This port's index stores structural metadata (symbols/imports/calls),
/// not raw source, so a file's path and symbol list is the closest thing
/// to a "document" it has.
fn file_document(root: &Path, file: &FileRecord) -> String {
    let rel = relative(root, &file.path);
    let mut doc = format!("File: {rel}\n");
    for sym in &file.symbols {
        doc.push_str(&format!("- {} ({})\n", sym.name, sym.kind.label()));
    }
    doc
}

/// Embed the question and every file, rank by cosine similarity, take
/// the top [`CONTEXT_LIMIT`].
///
/// **Re-embeds the whole corpus on every call.** There is no vector
/// index or persistence yet (issue #302), which is a defensible cost for
/// an occasional question and a bad one for anything frequent. Callers
/// that run this per keystroke will regret it.
///
/// `Ok(None)` only for an empty index. Any embeddings failure propagates
/// so the caller can fall back deliberately rather than silently.
fn semantic(
    root: &Path,
    index: &RepoIndex,
    question: &str,
    config: &LlmConfig,
) -> anyhow::Result<Option<Retrieval>> {
    if index.files.is_empty() {
        return Ok(None);
    }

    let mut inputs = vec![question.to_string()];
    inputs.extend(index.files.iter().map(|f| file_document(root, f)));
    let embeddings = crate::embed(config, &inputs)?;
    let mut embeddings = embeddings.into_iter();
    let question_embedding = embeddings
        .next()
        .ok_or_else(|| anyhow::anyhow!("embeddings response was empty"))?;

    let mut scored: Vec<(f32, &FileRecord)> = index
        .files
        .iter()
        .zip(embeddings)
        .map(|(file, embedding)| {
            (
                crate::cosine_similarity(&question_embedding, &embedding),
                file,
            )
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(CONTEXT_LIMIT);

    let mut context = String::from(PREAMBLE);
    context.push_str(&format!(
        "This repo has {} indexed file(s).\n",
        index.files.len()
    ));
    context.push_str(
        "\nPossibly relevant, found via semantic (embedding) search over indexed files:\n",
    );
    let mut cited = Vec::new();
    for (score, file) in &scored {
        let rel = relative(root, &file.path);
        context.push_str(&format!("- File: {rel} (similarity {score:.2})\n"));
        for sym in &file.symbols {
            context.push_str(&format!("  - {} ({})\n", sym.name, sym.kind.label()));
        }
        cited.push(rel);
    }

    Ok(Some(Retrieval {
        context,
        cited,
        mode: RetrievalMode::Semantic,
    }))
}

/// Substring matching over paths and symbol names. The fallback.
fn keyword(root: &Path, index: &RepoIndex, question: &str) -> Retrieval {
    let mut context = String::from(PREAMBLE);
    context.push_str(&format!(
        "This repo has {} indexed file(s).\n",
        index.files.len()
    ));

    let words: Vec<String> = question
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() > 2)
        .collect();

    let mut matches = Vec::new();
    let mut cited = Vec::new();
    if !words.is_empty() {
        for file in &index.files {
            let rel = relative(root, &file.path);
            let rel_lower = rel.to_lowercase();
            let mut hit = false;
            if words.iter().any(|w| rel_lower.contains(w.as_str())) {
                matches.push(format!("File: {rel}"));
                hit = true;
            }
            for sym in &file.symbols {
                let name_lower = sym.name.to_lowercase();
                if words.iter().any(|w| name_lower.contains(w.as_str())) {
                    matches.push(format!(
                        "Symbol: {} ({}) in {rel}:{}",
                        sym.name,
                        sym.kind.label(),
                        sym.start_line
                    ));
                    hit = true;
                }
            }
            if hit && !cited.contains(&rel) {
                cited.push(rel);
            }
        }
    }
    matches.truncate(CONTEXT_LIMIT);
    cited.truncate(CONTEXT_LIMIT);

    if matches.is_empty() {
        context.push_str("\nNo specific files or symbols matched keywords in the question.\n");
    } else {
        context.push_str(
            "\nPossibly relevant, found via keyword search over file paths and symbol names:\n",
        );
        for m in &matches {
            context.push_str(&format!("- {m}\n"));
        }
    }

    Retrieval {
        context,
        cited,
        mode: RetrievalMode::Keyword,
    }
}

/// Build context for `question`, preferring embeddings and falling back
/// to keyword matching.
///
/// The fallback is deliberate and reported rather than hidden -- see
/// [`RetrievalMode`].
pub fn retrieve(root: &Path, index: &RepoIndex, question: &str, config: &LlmConfig) -> Retrieval {
    match semantic(root, index, question, config) {
        Ok(Some(retrieval)) => retrieval,
        // Empty index, or embeddings unavailable. Either way the caller
        // gets a usable context and an honest `mode`.
        Ok(None) | Err(_) => keyword(root, index, question),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A real index over a real file, built with the real parser --
    /// hand-writing `Symbol` literals here would pin two dozen unrelated
    /// fields and rot on the next metric added.
    fn indexed(dir: &tempfile::TempDir) -> (PathBuf, RepoIndex) {
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/auth.rs"),
            "pub fn validate_token() -> bool { true }\n",
        )
        .unwrap();
        let index = repowise_parser::build_index(&root).unwrap();
        (root, index)
    }

    #[test]
    fn keyword_retrieval_cites_the_files_it_matched() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = indexed(&dir);
        let r = keyword(&root, &index, "how does validate_token work");
        assert_eq!(r.mode, RetrievalMode::Keyword);
        assert_eq!(r.cited, vec!["src/auth.rs"]);
        assert!(r.found_sources());
        assert!(r.context.contains("validate_token"), "{}", r.context);
    }

    /// Finding nothing is a real outcome, and must be detectable without
    /// reading the prose.
    #[test]
    fn no_match_produces_no_citations_rather_than_a_false_one() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = indexed(&dir);
        let r = keyword(&root, &index, "quantum chromodynamics");
        assert!(r.cited.is_empty());
        assert!(!r.found_sources());
        assert!(r.context.contains("No specific files or symbols matched"));
    }

    /// A file matching on both its path and a symbol is cited once.
    #[test]
    fn a_file_is_cited_once_however_many_times_it_matches() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = indexed(&dir);
        let r = keyword(&root, &index, "auth validate_token");
        assert_eq!(r.cited, vec!["src/auth.rs"]);
    }

    #[test]
    fn citations_are_repo_relative() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = indexed(&dir);
        let r = keyword(&root, &index, "auth");
        assert!(
            r.cited.iter().all(|c| !c.starts_with('/')),
            "citations must not leak absolute paths: {:?}",
            r.cited
        );
    }

    /// The degraded path has to announce itself; the healthy one must
    /// not, or the caveat becomes noise.
    #[test]
    fn only_the_keyword_mode_carries_a_caveat() {
        assert!(RetrievalMode::Keyword.caveat().is_some());
        assert_eq!(RetrievalMode::Semantic.caveat(), None);
        assert!(RetrievalMode::Keyword
            .caveat()
            .unwrap()
            .contains("retrieval failure"));
    }

    /// An unreachable endpoint must degrade to keyword rather than
    /// erroring: a weaker answer beats no answer, as long as it says so.
    #[test]
    fn an_unreachable_endpoint_falls_back_to_keyword() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = indexed(&dir);
        let config = LlmConfig {
            base_url: "http://127.0.0.1:1/unreachable".to_string(),
            model: "m".to_string(),
            embedding_model: "e".to_string(),
            api_key: None,
        };
        let r = retrieve(&root, &index, "auth", &config);
        assert_eq!(r.mode, RetrievalMode::Keyword);
        assert_eq!(r.cited, vec!["src/auth.rs"]);
    }

    #[test]
    fn an_empty_index_still_returns_usable_context() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let index = repowise_parser::build_index(&root).unwrap();
        let r = keyword(&root, &index, "anything");
        assert!(r.cited.is_empty());
        assert!(r.context.contains("0 indexed file(s)"));
    }
}
