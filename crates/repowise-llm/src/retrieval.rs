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
//!
//! # Reuse from the stored index, top up the rest
//!
//! Vectors come from [`crate::embedding_index`] when they're there, and
//! only the files it doesn't cover get embedded -- in the same single
//! batched call that also embeds the question. Three consequences worth
//! being explicit about:
//!
//! **Cost is never worse than re-embedding everything, and usually much
//! better.** One HTTP round trip either way; a fully covered index means
//! one input instead of N+1.
//!
//! **Coverage at answer time is always complete.** A partially covered
//! index would otherwise ground an answer in a *subset* of the repo
//! while its citations looked complete -- and unlike a ranking, an
//! answer gives the reader no way to see that. Topping up means it
//! can't happen, so there is nothing here to caveat. Prevented rather
//! than reported, the same shape as the embedding index's own staleness
//! guarantee.
//!
//! **Nothing is written.** `init`/`update` own the stored index; this is
//! a read path and stays one. Top-up vectors live for the call and are
//! discarded, so there's no race between concurrent questions and no
//! failure on a read-only checkout. The cost of not persisting is that
//! an index `update` never filled stays unfilled -- capped, as above, at
//! what this used to do every time anyway.

use crate::embedding_index::{self, EmbeddingIndex};
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

/// Where this call's vectors came from.
///
/// A **performance** fact, not a warning: coverage is complete either
/// way, because whatever the stored index lacks is embedded on the
/// spot. It exists so "why was that question slow" is answerable
/// without guessing, and so a repo that never ran `update` with an
/// embedding endpoint can find that out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VectorSources {
    /// Files whose vectors came from the stored index.
    pub reused: usize,
    /// Files embedded during this call because the index didn't cover
    /// them. Excludes the question, which is always embedded fresh.
    pub embedded_now: usize,
}

impl VectorSources {
    /// Nothing had to be embedded — the stored index covered every file.
    pub fn fully_reused(&self) -> bool {
        self.embedded_now == 0
    }

    /// Nothing was reused. Either no index exists, or it was built with
    /// a different embedding model and correctly ignored.
    pub fn nothing_reused(&self) -> bool {
        self.reused == 0
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
    /// Stored-vs-fresh vector counts. `None` for keyword retrieval,
    /// which embeds nothing — a zeroed struct there would read as "an
    /// index covered nothing" rather than "vectors weren't involved".
    pub vectors: Option<VectorSources>,
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

/// Embed the question, reuse stored file vectors, embed only what's
/// missing, then rank by cosine similarity and take the top
/// [`CONTEXT_LIMIT`].
///
/// The document text comes from [`embedding_index::document`] rather
/// than a local copy. That is not tidiness: entries in the stored index
/// are keyed by a hash of the exact text embedded, so a second
/// definition that drifted by one character would produce a total cache
/// miss with no error to notice. There is one definition, and this is a
/// caller of it.
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

    // `None` for no index *or* an index built with a different embedding
    // model -- the second is deliberate: vectors across models aren't
    // comparable, so everything gets embedded fresh rather than mixed.
    let stored = EmbeddingIndex::load(root, &config.embedding_model);

    // The question always needs embedding; it isn't in any index.
    let mut inputs = vec![question.to_string()];
    // Per file: the stored vector, or a slot to fill from the response.
    let mut resolved: Vec<Option<Vec<f32>>> = Vec::with_capacity(index.files.len());
    let mut sources = VectorSources::default();

    for file in &index.files {
        match stored.as_ref().and_then(|s| s.vector_for(root, file)) {
            Some(vector) => {
                sources.reused += 1;
                resolved.push(Some(vector.clone()));
            }
            None => {
                sources.embedded_now += 1;
                inputs.push(embedding_index::document(root, file));
                resolved.push(None);
            }
        }
    }

    let embeddings = crate::embed(config, &inputs)?;
    if embeddings.len() != inputs.len() {
        // Same guard the index refresh uses: a misaligned response would
        // pair each file with another file's vector, which ranks
        // confidently and wrongly rather than failing.
        anyhow::bail!(
            "embeddings response had {} vector(s) for {} input(s) -- refusing to pair \
             them up",
            embeddings.len(),
            inputs.len()
        );
    }
    let mut embeddings = embeddings.into_iter();
    let question_embedding = embeddings
        .next()
        .ok_or_else(|| anyhow::anyhow!("embeddings response was empty"))?;

    // Fill the gaps in file order -- `inputs` was built in that order, so
    // the remaining response entries line up with the `None` slots.
    let mut fresh = embeddings;
    let mut scored: Vec<(f32, &FileRecord)> = Vec::with_capacity(index.files.len());
    for (file, slot) in index.files.iter().zip(resolved) {
        let vector = match slot {
            Some(stored) => stored,
            None => fresh
                .next()
                .ok_or_else(|| anyhow::anyhow!("embeddings response was short"))?,
        };
        scored.push((crate::cosine_similarity(&question_embedding, &vector), file));
    }
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
        vectors: Some(sources),
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
        // Not zero — absent. Nothing was embedded here at all, and a
        // zeroed count would read as an index that covered nothing.
        vectors: None,
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

    /// Two files, so a stored index can cover one of them and leave the
    /// other for top-up to pick up.
    fn indexed_two_files(dir: &tempfile::TempDir) -> (PathBuf, RepoIndex) {
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/auth.rs"),
            "pub fn validate_token() -> bool { true }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/config.rs"),
            "pub fn load_config() -> u8 { 0 }\n",
        )
        .unwrap();
        let index = repowise_parser::build_index(&root).unwrap();
        (root, index)
    }

    /// A fixture endpoint that records the raw request body it received.
    ///
    /// The existing `FixtureServer` in this crate's top-level tests only
    /// checks response parsing; it can't catch the bug this feature
    /// exists to prevent -- silently re-embedding a file the stored
    /// index already covers. That requires inspecting *what was sent*,
    /// not just how many vectors came back, since a request with the
    /// wrong document count still gets a same-length response and looks
    /// fine from the response side alone.
    struct CapturingServer {
        addr: std::net::SocketAddr,
        body: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    }

    impl CapturingServer {
        fn start(response: String) -> Self {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let body = std::sync::Arc::new(std::sync::Mutex::new(None));
            let captured = body.clone();
            std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();

                // Read until the header/body separator shows up, then
                // keep reading until `Content-Length` bytes of body have
                // arrived -- a single `read()` call is not guaranteed to
                // return the whole request in one go, and this request
                // is bigger than the usual fixture responses in this
                // crate's other tests.
                let mut received: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 4096];
                let header_end = loop {
                    let n = stream.read(&mut chunk).unwrap();
                    assert!(n > 0, "connection closed before headers completed");
                    received.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = find_subslice(&received, b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&received[..header_end]).to_string();
                let content_length: usize = headers
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().to_string())
                    })
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                while received.len() - header_end < content_length {
                    let n = stream.read(&mut chunk).unwrap();
                    assert!(n > 0, "connection closed before the full body arrived");
                    received.extend_from_slice(&chunk[..n]);
                }
                let payload =
                    String::from_utf8_lossy(&received[header_end..header_end + content_length])
                        .to_string();
                *captured.lock().unwrap() = Some(payload);

                let http = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                let _ = stream.write_all(http.as_bytes());
            });
            CapturingServer { addr, body }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        /// The `input` array the embeddings request actually carried.
        fn requested_inputs(&self) -> Vec<String> {
            let raw = self
                .body
                .lock()
                .unwrap()
                .clone()
                .expect("no request was received");
            let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
            json["input"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        }
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn embeddings_response(count: usize) -> String {
        let data: Vec<String> = (0..count)
            .map(|i| format!(r#"{{"embedding": [{}, {}]}}"#, i as f32 + 1.0, 0.0))
            .collect();
        format!(r#"{{"data": [{}]}}"#, data.join(","))
    }

    /// The core guarantee: a file already in the stored index must not
    /// be re-embedded. Verified by inspecting the actual request, not
    /// just the returned counts -- see `CapturingServer`'s doc comment.
    #[test]
    fn a_file_already_in_the_stored_index_is_not_re_embedded() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = indexed_two_files(&dir);
        let auth = index
            .files
            .iter()
            .find(|f| f.path.ends_with("auth.rs"))
            .unwrap();
        let config_file = index
            .files
            .iter()
            .find(|f| f.path.ends_with("config.rs"))
            .unwrap();

        let mut stored = EmbeddingIndex::new("embed");
        stored.entries.insert(
            embedding_index::document_key(&embedding_index::document(&root, auth)),
            vec![1.0, 0.0],
        );
        stored.save(&root).unwrap();

        // The question, plus exactly one file -- config.rs, the one
        // that isn't in `stored`.
        let server = CapturingServer::start(embeddings_response(2));
        let config = LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let r = retrieve(&root, &index, "how does auth work", &config);

        assert_eq!(r.mode, RetrievalMode::Semantic);
        assert_eq!(
            r.vectors,
            Some(VectorSources {
                reused: 1,
                embedded_now: 1,
            })
        );

        let sent = server.requested_inputs();
        assert_eq!(sent.len(), 2, "question + exactly one file: {sent:?}");
        let auth_doc = embedding_index::document(&root, auth);
        let config_doc = embedding_index::document(&root, config_file);
        assert!(
            !sent.contains(&auth_doc),
            "auth.rs was already in the stored index and must not be re-embedded: {sent:?}"
        );
        assert!(
            sent.contains(&config_doc),
            "config.rs has no stored vector and must be embedded: {sent:?}"
        );

        // Both files still rank, even though only one was embedded this
        // call -- the reused vector is used for real, not dropped.
        assert_eq!(r.cited.len(), 2);
    }

    /// A fully covered index costs one input: the question. Nothing
    /// else gets embedded.
    #[test]
    fn a_fully_covered_index_embeds_only_the_question() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = indexed_two_files(&dir);

        let mut stored = EmbeddingIndex::new("embed");
        for file in &index.files {
            stored.entries.insert(
                embedding_index::document_key(&embedding_index::document(&root, file)),
                vec![1.0, 0.0],
            );
        }
        stored.save(&root).unwrap();

        let server = CapturingServer::start(embeddings_response(1));
        let config = LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let r = retrieve(&root, &index, "how does auth work", &config);

        assert_eq!(
            r.vectors,
            Some(VectorSources {
                reused: 2,
                embedded_now: 0,
            })
        );
        assert!(r.vectors.unwrap().fully_reused());
        let sent = server.requested_inputs();
        assert_eq!(
            sent,
            vec!["how does auth work".to_string()],
            "a fully covered index must send nothing but the question: {sent:?}"
        );
    }

    /// No stored index at all: every file falls through to top-up,
    /// which is exactly today's (pre-#308) behavior -- this is the
    /// worst case, not a regression.
    #[test]
    fn with_no_stored_index_every_file_is_embedded_this_call() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = indexed_two_files(&dir);

        let server = CapturingServer::start(embeddings_response(3));
        let config = LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let r = retrieve(&root, &index, "how does auth work", &config);

        assert_eq!(
            r.vectors,
            Some(VectorSources {
                reused: 0,
                embedded_now: 2,
            })
        );
        assert!(r.vectors.unwrap().nothing_reused());
        assert_eq!(server.requested_inputs().len(), 3);
    }

    /// Keyword retrieval embeds nothing, so its vector counts must be
    /// absent, not zero -- a zeroed struct there would misread as "an
    /// index covered nothing" rather than "vectors weren't involved".
    #[test]
    fn keyword_retrieval_carries_no_vector_counts() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = indexed(&dir);
        let r = keyword(&root, &index, "auth");
        assert_eq!(r.vectors, None);
    }
}
