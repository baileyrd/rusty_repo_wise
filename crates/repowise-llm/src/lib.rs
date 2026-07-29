//! Optional, opt-in LLM-assisted features — the one crate in this port
//! that talks to an LLM. Every other crate is deliberately deterministic
//! (see `repowise-docs`'s own module doc comment); this crate exists so
//! determinism stays the default everywhere else while still letting an
//! operator opt into an LLM-written layer on top of it.
//!
//! Speaks the OpenAI-compatible chat-completions wire format
//! (`POST {base_url}/v1/chat/completions`) against any compatible
//! endpoint — including a self-hosted `rusty_provider` instance
//! (<https://github.com/baileyrd/rusty_provider>), which fronts
//! OpenAI/Anthropic/Gemini/Groq/Together/Fireworks behind one URL with
//! config-driven fallback chains, or any other OpenAI-compatible server.
//!
//! Entirely opt-in via environment variables (mirroring
//! `REPOWISE_GITHUB_TOKEN`'s "unset = feature off" pattern elsewhere in
//! this port): unset `REPOWISE_LLM_BASE_URL` and every LLM feature here
//! degrades to "not available" rather than failing.
//!
//! This is a first, deliberately narrow slice of the four LLM-dependent
//! features tracked by issue #61 (wiki-prose generation, RAG chat,
//! refactor-plan codegen, doc-gen-as-decision-source): only wiki-summary
//! generation is implemented here. The other three need real
//! retrieval/context design of their own and are left as follow-ups.
//!
//! [`complete_messages_with_usage`] also returns each response's token
//! [`Usage`] (`prompt_tokens`/`completion_tokens`/`total_tokens`), when
//! the endpoint reports it -- `repowise-server`'s `/api/usage` (issue
//! #65's cost-tracking view) tallies these across chat calls. Token
//! counts only, not a dollar cost: this crate has no per-model pricing
//! table, since `rusty_provider` (and any other OpenAI-compatible
//! endpoint) can route to whichever provider it's configured for.
//!
//! [`embed`] is issue #63's real semantic retrieval: an OpenAI-
//! compatible `POST {base_url}/v1/embeddings` call, plus
//! [`cosine_similarity`] as the pure-function ranking step --
//! `repowise-server`'s chat view embeds the question and each indexed
//! file's symbol list, then ranks by similarity, instead of the
//! keyword-substring grounding it used before. No vector index or
//! persistence: every chat call re-embeds the whole corpus in one
//! batched request, an honest cost/latency tradeoff for a first slice
//! (see `repowise-server`'s own module doc for the fallback behavior
//! when an endpoint doesn't support embeddings at all).

pub mod retrieval;

pub use retrieval::{retrieve, Retrieval, RetrievalMode};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// LLM endpoint configuration. Read from environment variables at the
/// call site (`from_env`), then passed around as a plain value so the
/// actual HTTP-calling logic stays a pure, testable unit — the same
/// "env var at the outer edge, plain parameter inside" shape
/// `repowise-git`/`repowise-adr`'s GitHub-token-gated features use.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    /// Model/route alias for [`embed`] -- a separate setting from
    /// `model` since embedding and chat models are never the same
    /// model in practice.
    pub embedding_model: String,
    pub api_key: Option<String>,
}

impl LlmConfig {
    /// `None` when `REPOWISE_LLM_BASE_URL` isn't set — the single on/off
    /// switch for every LLM feature in this crate. `REPOWISE_LLM_MODEL`
    /// defaults to `"smart"` (a plausible `rusty_provider` route alias);
    /// point it at a direct `"provider/model"` string or your own alias
    /// for any other OpenAI-compatible endpoint. `REPOWISE_EMBEDDING_MODEL`
    /// defaults to `"embed"`, the same "plausible alias" convention.
    /// `REPOWISE_LLM_API_KEY` is optional — omit it for an endpoint that
    /// doesn't require one.
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("REPOWISE_LLM_BASE_URL")
            .ok()
            .filter(|s| !s.is_empty())?;
        let model = std::env::var("REPOWISE_LLM_MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "smart".to_string());
        let embedding_model = std::env::var("REPOWISE_EMBEDDING_MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "embed".to_string());
        let api_key = std::env::var("REPOWISE_LLM_API_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        Some(LlmConfig {
            base_url,
            model,
            embedding_model,
            api_key,
        })
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage<'a>],
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: Option<UsageDto>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct UsageDto {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

/// Token counts for one completion, when the endpoint reports them --
/// the OpenAI-compatible `usage` object is optional per-provider, so
/// callers that want it get `None` rather than zeros for an endpoint
/// that omits it. Deliberately token counts, not a dollar cost: this
/// crate has no per-model pricing table (the "cost" varies by whichever
/// provider `rusty_provider` -- or any other endpoint -- routes to).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// One turn in a multi-turn conversation passed to [`complete_messages`].
/// `role` is `"system"`, `"user"`, or `"assistant"`, matching the
/// OpenAI-compatible wire format directly.
#[derive(Debug, Clone)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

impl Turn {
    pub fn system(content: impl Into<String>) -> Self {
        Turn {
            role: "system".to_string(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Turn {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Turn {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

/// One OpenAI-compatible chat-completions round trip over an arbitrary
/// turn history in, the assistant's reply text (and token usage, when
/// the endpoint reports it) out. Synchronous (`ureq`), matching the
/// HTTP-client choice `repowise-adr`/`repowise-git` already made for
/// their own opt-in network calls, so callers don't need to pull an
/// async runtime into an otherwise-synchronous context the way
/// `repowise serve` does.
pub fn complete_messages_with_usage(
    config: &LlmConfig,
    turns: &[Turn],
) -> anyhow::Result<(String, Option<Usage>)> {
    let url = format!(
        "{}/v1/chat/completions",
        config.base_url.trim_end_matches('/')
    );
    let messages: Vec<ChatMessage> = turns
        .iter()
        .map(|t| ChatMessage {
            role: &t.role,
            content: &t.content,
        })
        .collect();
    let body = ChatRequest {
        model: &config.model,
        messages: &messages,
    };
    let mut req = ureq::post(&url)
        .set("Content-Type", "application/json")
        .set("Connection", "close");
    if let Some(key) = &config.api_key {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    let response: ChatResponse = req.send_json(serde_json::to_value(&body)?)?.into_json()?;
    let usage = response.usage.map(|u| Usage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
    });
    let reply = response
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| anyhow::anyhow!("LLM response had no choices"))?;
    Ok((reply, usage))
}

/// Same round trip as [`complete_messages_with_usage`], for the callers
/// that only need the reply text.
pub fn complete_messages(config: &LlmConfig, turns: &[Turn]) -> anyhow::Result<String> {
    complete_messages_with_usage(config, turns).map(|(reply, _)| reply)
}

/// A single `system`/`user` round trip -- the shape every caller before
/// the chat view needed. Thin wrapper over [`complete_messages`].
pub fn complete(config: &LlmConfig, system: &str, user: &str) -> anyhow::Result<String> {
    complete_messages(config, &[Turn::system(system), Turn::user(user)])
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingDatum>,
}

#[derive(Deserialize)]
struct EmbeddingDatum {
    embedding: Vec<f32>,
}

/// One OpenAI-compatible embeddings round trip
/// (`POST {base_url}/v1/embeddings`), batching every string in `inputs`
/// into a single request -- the response preserves `inputs`' order, one
/// embedding vector per input. Empty `inputs` short-circuits to an
/// empty result without a network call.
pub fn embed(config: &LlmConfig, inputs: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!("{}/v1/embeddings", config.base_url.trim_end_matches('/'));
    let body = EmbeddingRequest {
        model: &config.embedding_model,
        input: inputs,
    };
    let mut req = ureq::post(&url)
        .set("Content-Type", "application/json")
        .set("Connection", "close");
    if let Some(key) = &config.api_key {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    let response: EmbeddingResponse = req.send_json(serde_json::to_value(&body)?)?.into_json()?;
    Ok(response.data.into_iter().map(|d| d.embedding).collect())
}

/// Cosine similarity between two embedding vectors, in `[-1.0, 1.0]`
/// (`0.0` for a mismatched length or a zero vector, rather than a NaN
/// or a panic) -- the ranking step for embeddings-based retrieval.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

const SUMMARY_SYSTEM_PROMPT: &str = "You are writing a short summary for a code wiki page. \
Given the page's deterministic content (symbol list, dependencies, health findings), write a \
plain-English summary of what this file does and why it might matter to a reader, in 2-3 \
sentences. Do not repeat the raw data verbatim -- synthesize it into prose. Do not use markdown \
headings.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryStatus {
    /// A summary was generated and written into the wiki page.
    Written,
    /// No wiki page exists for this file yet -- run `repowise docs` first.
    NoWikiPage,
    /// The LLM call (or the page rewrite) failed; the page is left
    /// untouched rather than partially overwritten.
    Failed,
}

#[derive(Debug, Clone)]
pub struct SummaryResult {
    pub source_file: PathBuf,
    pub status: SummaryStatus,
}

/// For every indexed file with an existing wiki page, ask the LLM for a
/// short summary of that page's already-deterministic content and
/// insert it as a "## Summary" section right after the page's title —
/// requires `repowise docs` to have been run at some point (same
/// "augment, don't generate" relationship `repowise-dashboard`'s
/// drill-down links have with wiki pages). One file's failure (missing
/// page, LLM error) doesn't stop the rest; each file gets its own
/// `SummaryResult`. Re-running replaces a previous summary rather than
/// stacking a second one.
pub fn generate_wiki_summaries(
    index: &repowise_core::RepoIndex,
    config: &LlmConfig,
) -> Vec<SummaryResult> {
    index
        .files
        .iter()
        .map(|file| {
            let wiki_path = repowise_docs::wiki_page_path(&index.root, &file.path);
            let Ok(page) = std::fs::read_to_string(&wiki_path) else {
                return SummaryResult {
                    source_file: file.path.clone(),
                    status: SummaryStatus::NoWikiPage,
                };
            };
            let status = match complete(config, SUMMARY_SYSTEM_PROMPT, &page) {
                Ok(summary) => {
                    let annotated = insert_summary(&page, &summary);
                    if std::fs::write(&wiki_path, annotated).is_ok() {
                        SummaryStatus::Written
                    } else {
                        SummaryStatus::Failed
                    }
                }
                Err(_) => SummaryStatus::Failed,
            };
            SummaryResult {
                source_file: file.path.clone(),
                status,
            }
        })
        .collect()
}

/// Insert `summary` as a "## Summary" section right before "## Symbols"
/// (the first heading every wiki page has, per
/// `repowise_docs::render::render_page`), replacing any summary section
/// a previous `generate` run already left there so re-running stays
/// idempotent rather than stacking duplicate sections.
fn insert_summary(page: &str, summary: &str) -> String {
    let cleaned = strip_existing_summary(page);
    match cleaned.find("## Symbols") {
        Some(idx) => format!(
            "{}## Summary\n\n{}\n\n{}",
            &cleaned[..idx],
            summary.trim(),
            &cleaned[idx..]
        ),
        None => cleaned,
    }
}

fn strip_existing_summary(page: &str) -> String {
    let Some(summary_idx) = page.find("## Summary") else {
        return page.to_string();
    };
    let Some(symbols_offset) = page[summary_idx..].find("## Symbols") else {
        return page.to_string();
    };
    let symbols_idx = summary_idx + symbols_offset;
    format!("{}{}", &page[..summary_idx], &page[symbols_idx..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{FileRecord, Language, RepoIndex};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Same hand-rolled fixture-server approach `repowise-adr`'s
    /// `pull_requests` tests and `repowise-git`'s `issue_refs` tests
    /// use: exercises the real request/response/JSON-parsing path with
    /// no live network call and no mocking crate.
    struct FixtureServer {
        addr: std::net::SocketAddr,
    }

    impl FixtureServer {
        fn start(responses: Vec<&'static str>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                for body in responses {
                    let (mut stream, _) = listener.accept().unwrap();
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
    fn complete_parses_the_real_http_and_json_path() {
        let response = r#"{"choices": [{"message": {"role": "assistant", "content": "It adds two numbers."}}]}"#;
        let server = FixtureServer::start(vec![response]);
        let config = LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let reply = complete(&config, "system prompt", "user prompt").unwrap();
        assert_eq!(reply, "It adds two numbers.");
    }

    #[test]
    fn complete_messages_handles_an_arbitrary_length_turn_history() {
        let response = r#"{"choices": [{"message": {"role": "assistant", "content": "Sure, here's more detail."}}]}"#;
        let server = FixtureServer::start(vec![response]);
        let config = LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let reply = complete_messages(
            &config,
            &[
                Turn::system("You are a helpful assistant."),
                Turn::user("What does this repo do?"),
                Turn::assistant("It's a code health analyzer."),
                Turn::user("Tell me more."),
            ],
        )
        .unwrap();
        assert_eq!(reply, "Sure, here's more detail.");
    }

    #[test]
    fn complete_messages_with_usage_captures_reported_token_counts() {
        let response = r#"{"choices": [{"message": {"role": "assistant", "content": "Hi."}}], "usage": {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15}}"#;
        let server = FixtureServer::start(vec![response]);
        let config = LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let (reply, usage) = complete_messages_with_usage(&config, &[Turn::user("hi")]).unwrap();

        assert_eq!(reply, "Hi.");
        let usage = usage.expect("usage should be present");
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 3);
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn complete_messages_with_usage_is_none_when_the_endpoint_omits_it() {
        let response = r#"{"choices": [{"message": {"role": "assistant", "content": "Hi."}}]}"#;
        let server = FixtureServer::start(vec![response]);
        let config = LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let (_, usage) = complete_messages_with_usage(&config, &[Turn::user("hi")]).unwrap();

        assert!(usage.is_none());
    }

    #[test]
    fn embed_parses_a_batched_embeddings_response_in_order() {
        let response = r#"{"data": [{"embedding": [1.0, 0.0]}, {"embedding": [0.0, 1.0]}]}"#;
        let server = FixtureServer::start(vec![response]);
        let config = LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let embeddings = embed(&config, &["a".to_string(), "b".to_string()]).unwrap();

        assert_eq!(embeddings, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    }

    #[test]
    fn embed_of_empty_input_short_circuits_without_a_network_call() {
        let config = LlmConfig {
            base_url: "http://127.0.0.1:1".to_string(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let embeddings = embed(&config, &[]).unwrap();

        assert!(embeddings.is_empty());
    }

    #[test]
    fn cosine_similarity_of_identical_vectors_is_one() {
        assert!((cosine_similarity(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_of_orthogonal_vectors_is_zero() {
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_of_opposite_vectors_is_negative_one() {
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_is_zero_for_a_zero_vector_or_mismatched_lengths() {
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn insert_summary_places_a_section_right_before_symbols() {
        let page = "<!-- content-hash: 1 -->\n# a.rs\n\n**Language:** Rust  \n**Lines:** 3\n\n## Symbols\n\n_No symbols indexed._\n\n## Health\n\n_No findings._\n";

        let annotated = insert_summary(page, "A short summary.");
        assert!(annotated.contains("## Summary\n\nA short summary.\n\n## Symbols"));
    }

    #[test]
    fn insert_summary_replaces_rather_than_stacks_a_previous_one() {
        let page = "# a.rs\n\n## Summary\n\nOld summary.\n\n## Symbols\n\n_No symbols indexed._\n";

        let annotated = insert_summary(page, "New summary.");
        assert_eq!(annotated.matches("## Summary").count(), 1);
        assert!(annotated.contains("New summary."));
        assert!(!annotated.contains("Old summary."));
    }

    #[test]
    fn generate_wiki_summaries_writes_for_existing_pages_and_flags_missing_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let has_page = root.join("has_page.rs");
        let no_page = root.join("no_page.rs");

        let wiki_dir = root.join(".repowise").join("wiki");
        std::fs::create_dir_all(&wiki_dir).unwrap();
        std::fs::write(
            wiki_dir.join("has_page.rs.md"),
            "# has_page.rs\n\n## Symbols\n\n_No symbols indexed._\n",
        )
        .unwrap();

        let index = RepoIndex {
            root: root.clone(),
            files: vec![
                FileRecord {
                    path: has_page.clone(),
                    language: Language::Rust,
                    lines: 1,
                    symbols: vec![],
                    imports: vec![],
                    calls: vec![],
                    field_accesses: vec![],
                },
                FileRecord {
                    path: no_page.clone(),
                    language: Language::Rust,
                    lines: 1,
                    symbols: vec![],
                    imports: vec![],
                    calls: vec![],
                    field_accesses: vec![],
                },
            ],
            other_files: 0,
            indexed_commit: None,
        };

        let response =
            r#"{"choices": [{"message": {"role": "assistant", "content": "Generated summary."}}]}"#;
        let server = FixtureServer::start(vec![response]);
        let config = LlmConfig {
            base_url: server.base_url(),
            model: "smart".to_string(),
            embedding_model: "embed".to_string(),
            api_key: None,
        };

        let results = generate_wiki_summaries(&index, &config);

        let has_page_result = results.iter().find(|r| r.source_file == has_page).unwrap();
        assert_eq!(has_page_result.status, SummaryStatus::Written);
        let written = std::fs::read_to_string(wiki_dir.join("has_page.rs.md")).unwrap();
        assert!(written.contains("Generated summary."));

        let no_page_result: Vec<&SummaryResult> = results
            .iter()
            .filter(|r| r.source_file == no_page)
            .collect();
        assert_eq!(no_page_result.len(), 1);
        assert_eq!(no_page_result[0].status, SummaryStatus::NoWikiPage);
    }
}
