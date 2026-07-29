//! A persisted embedding index, so semantic search doesn't re-embed the
//! world on every query.
//!
//! # Staleness is prevented, not detected
//!
//! A stale embedding is worse than a missing one: it returns confident,
//! wrong neighbours, and nothing about the result looks wrong. So rather
//! than storing a timestamp or a commit and comparing it — which leaves
//! a window where the comparison is right and the vector isn't — entries
//! are **keyed by a hash of the exact text that was embedded**.
//!
//! If a file changes such that its document text changes, its key
//! changes, and the old vector is simply not found. There is no
//! comparison to get wrong and no window to be stale in. The same
//! content-addressing idea `repowise-distill`'s omission store uses.
//!
//! # The model is part of the identity
//!
//! Vectors from different embedding models are not comparable — cosine
//! similarity between them is noise that looks like a score. The index
//! records which model produced it, and a mismatch invalidates the whole
//! thing rather than silently mixing.
//!
//! # Partial coverage is reported
//!
//! An index built before some files existed covers only what it covers.
//! A search over 60% of a repo that presents itself as a search over the
//! repo is the failure this reports its way out of: [`Coverage`] carries
//! both numbers so a caller can say so.

use crate::LlmConfig;
use repowise_core::{FileRecord, RepoIndex};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const EMBEDDINGS_FILE: &str = "embeddings.json";

/// A stored embedding index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingIndex {
    /// The embedding model that produced every vector here. A different
    /// model invalidates all of them.
    pub model: String,
    /// Document-hash to vector.
    pub entries: BTreeMap<String, Vec<f32>>,
}

/// How much of the current repo the index actually covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    pub embedded: usize,
    pub total: usize,
}

impl Coverage {
    pub fn is_complete(&self) -> bool {
        self.embedded == self.total
    }

    /// `None` for an empty repo — a coverage percentage over zero files
    /// is not 100%, it's undefined, and reporting it as complete would
    /// be a small lie in the direction that matters.
    pub fn percent(&self) -> Option<f64> {
        (self.total > 0).then(|| self.embedded as f64 / self.total as f64 * 100.0)
    }
}

pub fn index_path(root: &Path) -> PathBuf {
    root.join(repowise_core::RepoIndex::INDEX_DIR)
        .join(EMBEDDINGS_FILE)
}

/// The exact text embedded for a file. Must stay identical between
/// building and querying, or keys won't match.
pub fn document(root: &Path, file: &FileRecord) -> String {
    let rel = file
        .path
        .strip_prefix(root)
        .unwrap_or(&file.path)
        .display()
        .to_string();
    let mut doc = format!("File: {rel}\n");
    for sym in &file.symbols {
        doc.push_str(&format!("- {} ({})\n", sym.name, sym.kind.label()));
    }
    doc
}

/// Content key for a document.
///
/// Retrieval is by key, so this never has to be stable across toolchain
/// versions — a changed hash function just invalidates the index, which
/// is the safe direction. Same reasoning as the omission store's refs.
pub fn document_key(document: &str) -> String {
    let mut hasher = DefaultHasher::new();
    document.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

impl EmbeddingIndex {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            entries: BTreeMap::new(),
        }
    }

    /// Load the index, if one exists and was built with `model`.
    ///
    /// A model mismatch returns `None` — the vectors are real but
    /// meaningless against a different model's query vector, which is
    /// exactly the kind of plausible-looking wrongness worth refusing.
    pub fn load(root: &Path, model: &str) -> Option<Self> {
        let raw = std::fs::read_to_string(index_path(root)).ok()?;
        let index: Self = serde_json::from_str(&raw).ok()?;
        (index.model == model).then_some(index)
    }

    pub fn save(&self, root: &Path) -> anyhow::Result<PathBuf> {
        let path = index_path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string(self)?)?;
        Ok(path)
    }

    pub fn size_bytes(&self, root: &Path) -> u64 {
        std::fs::metadata(index_path(root))
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// How much of `index` this covers.
    pub fn coverage(&self, root: &Path, index: &RepoIndex) -> Coverage {
        let embedded = index
            .files
            .iter()
            .filter(|f| self.entries.contains_key(&document_key(&document(root, f))))
            .count();
        Coverage {
            embedded,
            total: index.files.len(),
        }
    }

    pub fn vector_for(&self, root: &Path, file: &FileRecord) -> Option<&Vec<f32>> {
        self.entries.get(&document_key(&document(root, file)))
    }
}

/// What a refresh did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshReport {
    /// Files whose vectors were reused because their content hadn't
    /// changed. The reason this exists at all.
    pub reused: usize,
    /// Files newly embedded this run.
    pub embedded: usize,
    /// Entries dropped because no current file produces that key.
    pub evicted: usize,
}

/// Build or refresh the index for `repo`.
///
/// Only embeds documents whose key isn't already stored, so an unchanged
/// repo costs one no-op call and a changed one costs only the diff.
/// Entries no longer produced by any file are dropped, so a deleted or
/// heavily edited file doesn't leave its vector behind forever.
pub fn refresh(
    root: &Path,
    repo: &RepoIndex,
    config: &LlmConfig,
) -> anyhow::Result<(EmbeddingIndex, RefreshReport)> {
    let existing = EmbeddingIndex::load(root, &config.embedding_model);
    let mut entries = existing.map(|i| i.entries).unwrap_or_default();

    let wanted: Vec<(String, String)> = repo
        .files
        .iter()
        .map(|f| {
            let doc = document(root, f);
            (document_key(&doc), doc)
        })
        .collect();

    let missing: Vec<(String, String)> = wanted
        .iter()
        .filter(|(key, _)| !entries.contains_key(key))
        .cloned()
        .collect();

    let embedded = missing.len();
    if !missing.is_empty() {
        let docs: Vec<String> = missing.iter().map(|(_, doc)| doc.clone()).collect();
        let vectors = crate::embed(config, &docs)?;
        if vectors.len() != missing.len() {
            anyhow::bail!(
                "embeddings response had {} vector(s) for {} document(s) -- refusing to \
                 pair them up, since a misaligned index returns confidently wrong \
                 neighbours",
                vectors.len(),
                missing.len()
            );
        }
        for ((key, _), vector) in missing.into_iter().zip(vectors) {
            entries.insert(key, vector);
        }
    }

    let live: std::collections::BTreeSet<&String> = wanted.iter().map(|(k, _)| k).collect();
    let before = entries.len();
    entries.retain(|k, _| live.contains(k));
    let evicted = before - entries.len();

    let index = EmbeddingIndex {
        model: config.embedding_model.clone(),
        entries,
    };
    let reused = wanted.len().saturating_sub(embedded);
    Ok((
        index,
        RefreshReport {
            reused,
            embedded,
            evicted,
        },
    ))
}

/// Why a semantic search couldn't run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// No LLM endpoint configured.
    NotConfigured,
    /// Configured, but nothing has been embedded yet.
    NoIndex,
    /// An index exists but was built with a different embedding model.
    ModelChanged { stored: String, current: String },
    /// Configured and indexed, but embedding the *query* failed.
    ///
    /// Distinct from [`Unavailable::NotConfigured`] because the fixes
    /// are unrelated: one is a setting, the other is an endpoint that
    /// is down or rejecting the request. Collapsing them would send
    /// someone to check an environment variable that is already right.
    EndpointFailed { detail: String },
}

impl Unavailable {
    pub fn explain(&self) -> String {
        match self {
            // Var names come from the constants `from_env` reads, not
            // from memory -- an earlier draft of this line invented
            // `REPOWISE_LLM_EMBEDDING_MODEL`, which does not exist.
            Unavailable::NotConfigured => format!(
                "semantic search needs an embedding endpoint: set {} (and {} if the \
                 default doesn't suit). Every other search mode works without it.",
                crate::BASE_URL_VAR,
                crate::EMBEDDING_MODEL_VAR
            ),
            Unavailable::NoIndex => "no embedding index has been built yet. Run `repowise \
                 update` with REPOWISE_LLM_BASE_URL set; it embeds each file once and \
                 reuses unchanged ones afterwards."
                .to_string(),
            Unavailable::ModelChanged { stored, current } => format!(
                "the stored embedding index was built with model {stored:?} but the \
                 configured model is now {current:?}. Vectors from different models \
                 aren't comparable -- similarity between them is noise that looks like a \
                 score -- so the index is being ignored rather than mixed. Re-run \
                 `repowise update` to rebuild it."
            ),
            Unavailable::EndpointFailed { detail } => format!(
                "the embedding endpoint is configured and an index exists, but embedding \
                 the query failed: {detail}. The stored index is fine -- this is the one \
                 live call semantic search still has to make."
            ),
        }
    }
}

/// One semantic search hit.
#[derive(Debug, Clone)]
pub struct Hit {
    pub file: PathBuf,
    pub similarity: f32,
}

/// Rank `repo`'s files against `query` using the stored index.
///
/// Embeds only the query — one short call — which is the entire point of
/// persisting the rest.
pub fn search(
    root: &Path,
    repo: &RepoIndex,
    query: &str,
    config: Option<&LlmConfig>,
) -> Result<(Vec<Hit>, Coverage), Unavailable> {
    let Some(config) = config else {
        return Err(Unavailable::NotConfigured);
    };

    // Distinguish "no index at all" from "an index for another model",
    // since the fixes differ.
    let stored_model = std::fs::read_to_string(index_path(root))
        .ok()
        .and_then(|raw| serde_json::from_str::<EmbeddingIndex>(&raw).ok())
        .map(|i| i.model);
    let index = match EmbeddingIndex::load(root, &config.embedding_model) {
        Some(index) => index,
        None => {
            return Err(match stored_model {
                Some(stored) => Unavailable::ModelChanged {
                    stored,
                    current: config.embedding_model.clone(),
                },
                None => Unavailable::NoIndex,
            })
        }
    };

    let coverage = index.coverage(root, repo);

    let query_vector = match crate::embed(config, &[query.to_string()]) {
        Ok(mut v) if !v.is_empty() => v.remove(0),
        Ok(_) => {
            return Err(Unavailable::EndpointFailed {
                detail: "the endpoint returned no vector for the query".to_string(),
            })
        }
        Err(e) => {
            return Err(Unavailable::EndpointFailed {
                detail: e.to_string(),
            })
        }
    };

    let mut hits: Vec<Hit> = repo
        .files
        .iter()
        .filter_map(|f| {
            index.vector_for(root, f).map(|v| Hit {
                file: f.path.clone(),
                similarity: crate::cosine_similarity(&query_vector, v),
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok((hits, coverage))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(dir: &tempfile::TempDir, files: &[(&str, &str)]) -> (PathBuf, RepoIndex) {
        let root = dir.path().canonicalize().unwrap();
        for (name, body) in files {
            let path = root.join(name);
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p).unwrap();
            }
            std::fs::write(path, body).unwrap();
        }
        let index = repowise_parser::build_index(&root).unwrap();
        (root, index)
    }

    /// The core guarantee: a changed file gets a different key, so its
    /// old vector can never be returned for it.
    #[test]
    fn changing_a_file_changes_its_key() {
        let dir = tempfile::tempdir().unwrap();
        let (root, before) = repo(&dir, &[("a.rs", "pub fn one() {}\n")]);
        let key_before = document_key(&document(&root, &before.files[0]));

        std::fs::write(root.join("a.rs"), "pub fn one() {}\npub fn two() {}\n").unwrap();
        let after = repowise_parser::build_index(&root).unwrap();
        let key_after = document_key(&document(&root, &after.files[0]));

        assert_ne!(
            key_before, key_after,
            "a changed file must not resolve to its old embedding"
        );
    }

    #[test]
    fn an_unchanged_file_keeps_its_key() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = repo(&dir, &[("a.rs", "pub fn one() {}\n")]);
        let first = document_key(&document(&root, &index.files[0]));
        let again = repowise_parser::build_index(&root).unwrap();
        assert_eq!(first, document_key(&document(&root, &again.files[0])));
    }

    /// Vectors from another model aren't comparable, so the whole index
    /// is refused rather than partially trusted.
    #[test]
    fn an_index_from_another_model_does_not_load() {
        let dir = tempfile::tempdir().unwrap();
        let (root, _) = repo(&dir, &[("a.rs", "pub fn one() {}\n")]);
        let mut stored = EmbeddingIndex::new("model-a");
        stored.entries.insert("k".to_string(), vec![1.0, 0.0]);
        stored.save(&root).unwrap();

        assert!(EmbeddingIndex::load(&root, "model-a").is_some());
        assert!(
            EmbeddingIndex::load(&root, "model-b").is_none(),
            "a different model's vectors must not be reused"
        );
    }

    #[test]
    fn search_distinguishes_no_index_from_a_model_change() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = repo(&dir, &[("a.rs", "pub fn one() {}\n")]);
        let config = LlmConfig {
            base_url: "http://127.0.0.1:1".to_string(),
            model: "m".to_string(),
            embedding_model: "model-b".to_string(),
            api_key: None,
        };

        assert_eq!(
            search(&root, &index, "q", Some(&config)).unwrap_err(),
            Unavailable::NoIndex
        );

        EmbeddingIndex::new("model-a").save(&root).unwrap();
        let err = search(&root, &index, "q", Some(&config)).unwrap_err();
        assert!(matches!(err, Unavailable::ModelChanged { .. }), "{err:?}");
        assert!(
            err.explain().contains("aren't comparable"),
            "{}",
            err.explain()
        );
    }

    #[test]
    fn search_without_config_says_it_is_unconfigured() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = repo(&dir, &[("a.rs", "pub fn one() {}\n")]);
        assert_eq!(
            search(&root, &index, "q", None).unwrap_err(),
            Unavailable::NotConfigured
        );
    }

    /// Partial coverage has to be visible: a search over most of a repo
    /// that presents as a search over the repo is the failure mode.
    #[test]
    fn coverage_reports_partial_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let (root, index) = repo(
            &dir,
            &[("a.rs", "pub fn one() {}\n"), ("b.rs", "pub fn two() {}\n")],
        );
        let mut stored = EmbeddingIndex::new("m");
        stored.entries.insert(
            document_key(&document(&root, &index.files[0])),
            vec![1.0, 0.0],
        );

        let coverage = stored.coverage(&root, &index);
        assert_eq!(coverage.embedded, 1);
        assert_eq!(coverage.total, 2);
        assert!(!coverage.is_complete());
        assert_eq!(coverage.percent(), Some(50.0));
    }

    /// 0 of 0 is undefined, not complete.
    #[test]
    fn coverage_of_an_empty_repo_has_no_percentage() {
        let coverage = Coverage {
            embedded: 0,
            total: 0,
        };
        assert_eq!(coverage.percent(), None);
    }

    fn all_reasons() -> Vec<Unavailable> {
        vec![
            Unavailable::NotConfigured,
            Unavailable::NoIndex,
            Unavailable::ModelChanged {
                stored: "a".into(),
                current: "b".into(),
            },
            Unavailable::EndpointFailed {
                detail: "connection refused".into(),
            },
        ]
    }

    #[test]
    fn every_unavailable_reason_explains_its_own_fix() {
        for reason in all_reasons() {
            let text = reason.explain();
            assert!(text.len() > 40, "{text}");
            assert!(
                text.contains("repowise update")
                    || text.contains(crate::BASE_URL_VAR)
                    || text.contains("endpoint"),
                "a reason should name the action that fixes it: {text}"
            );
        }
    }

    /// Every `REPOWISE_*` name an error message tells someone to set
    /// must be one `LlmConfig::from_env` actually reads.
    ///
    /// A shipped draft of `NotConfigured` named
    /// `REPOWISE_LLM_EMBEDDING_MODEL`, which does not exist — the real
    /// var has no `LLM_` infix. Setting it changes nothing, so the
    /// advice reads as correct and silently doesn't work.
    #[test]
    fn advice_never_names_an_env_var_that_does_not_exist() {
        let known = [
            crate::BASE_URL_VAR,
            crate::MODEL_VAR,
            crate::EMBEDDING_MODEL_VAR,
            crate::API_KEY_VAR,
        ];
        for reason in all_reasons() {
            let text = reason.explain();
            for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                if word.starts_with("REPOWISE_") {
                    assert!(
                        known.contains(&word),
                        "{word:?} is not a variable this crate reads -- \
                         known: {known:?} -- in: {text}"
                    );
                }
            }
        }
    }
}
