//! Shared data model and index persistence for repowise.
//!
//! This crate defines the on-disk index format and the language-agnostic
//! structures produced by `repowise-parser` and consumed by `repowise-graph`.

mod walk;

pub use walk::{discover_files, DiscoveredFile};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Languages the indexer understands. Unsupported files are still walked
/// (for stats) but are not parsed for symbols/edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Java,
    Kotlin,
    Go,
    Cpp,
    Other,
}

impl Language {
    pub fn from_extension(ext: &str) -> Self {
        match ext {
            "rs" => Language::Rust,
            "py" | "pyi" => Language::Python,
            "ts" | "tsx" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "java" => Language::Java,
            "kt" | "kts" => Language::Kotlin,
            "go" => Language::Go,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Language::Cpp,
            _ => Language::Other,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::Python => "Python",
            Language::TypeScript => "TypeScript",
            Language::JavaScript => "JavaScript",
            Language::Java => "Java",
            Language::Kotlin => "Kotlin",
            Language::Go => "Go",
            Language::Cpp => "C++",
            Language::Other => "Other",
        }
    }
}

/// Kind of a definition site extracted from source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Class,
    Module,
}

impl SymbolKind {
    pub fn label(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Class => "class",
            SymbolKind::Module => "module",
        }
    }
}

/// A unique, stable identifier for a symbol within a single indexing run.
/// Stable across runs as long as (file, name, start_line) doesn't change.
pub type SymbolId = String;

/// A function/struct/class/etc. definition discovered in a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    /// For methods: the enclosing struct/class/impl type name, if any.
    pub parent: Option<String>,
    /// McCabe cyclomatic complexity of the body (1 = no branching).
    /// `0` for symbols with no body to analyze (e.g. trait method
    /// signatures, structs/enums/traits/modules).
    pub complexity: usize,
    /// Number of declared parameters. `0` for symbols without a
    /// parameter list.
    pub param_count: usize,
    /// A hash of the body's whitespace-normalized text, used for
    /// best-effort duplicate-code detection. `None` when there's no body
    /// or the body is too short to be a meaningful duplicate signal.
    pub body_hash: Option<u64>,
}

impl Symbol {
    pub fn make_id(file: &Path, name: &str, start_line: usize) -> SymbolId {
        format!("{}::{}@{}", file.display(), name, start_line)
    }
}

/// A `use`/`import`/`from ... import ...` style reference, unresolved
/// unless `resolved_file` is already known at extraction time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRef {
    /// Dotted / `::`-separated module path as written in source, e.g.
    /// `crate::graph::build` or `os.path`.
    pub path: String,
    pub line: usize,
    /// Set when the extractor could resolve this reference directly from
    /// the filesystem (e.g. Rust's `mod foo;` maps deterministically to a
    /// sibling file), bypassing the module-index heuristic in
    /// `repowise-graph`.
    pub resolved_file: Option<PathBuf>,
}

/// A call-expression reference, unresolved. `caller` is the enclosing
/// symbol's id if the call happens inside a known symbol, else `None`
/// (e.g. a call at module scope).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRef {
    pub caller: Option<SymbolId>,
    /// The last path segment of the call target, e.g. `foo` in `bar.foo()`
    /// or `mod::foo()`.
    pub callee_name: String,
    pub line: usize,
}

/// Everything extracted from a single source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: PathBuf,
    pub language: Language,
    pub lines: usize,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<ImportRef>,
    pub calls: Vec<CallRef>,
}

/// The full index for a repository: one record per parsed file, plus
/// unparsed files counted only for stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIndex {
    pub root: PathBuf,
    pub files: Vec<FileRecord>,
    pub other_files: usize,
}

impl RepoIndex {
    pub const INDEX_DIR: &'static str = ".repowise";
    pub const INDEX_FILE: &'static str = "index.json";

    pub fn index_path(root: &Path) -> PathBuf {
        root.join(Self::INDEX_DIR).join(Self::INDEX_FILE)
    }

    pub fn save(&self, root: &Path) -> anyhow::Result<PathBuf> {
        let dir = root.join(Self::INDEX_DIR);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(Self::INDEX_FILE);
        let file = std::fs::File::create(&path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(path)
    }

    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = Self::index_path(root);
        let file = std::fs::File::open(&path).map_err(|e| {
            anyhow::anyhow!(
                "no index found at {} ({e}); run `repowise init` first",
                path.display()
            )
        })?;
        Ok(serde_json::from_reader(file)?)
    }
}
