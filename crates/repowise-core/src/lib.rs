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
///
/// The last 9 variants (ObjectiveC through D) are issue #70's
/// "Structural tier" -- recognized by name and given a real
/// (zero-symbol) `FileRecord` so they're visible to git-history-derived
/// views (`hotspots`, per-language file counts, dashboard drill-down),
/// but never parsed by tree-sitter at all: no grammar, no symbols,
/// always a `0` hotspot score (churn × 0 complexity). `ownership`/
/// `coupled` already work for these files today regardless of this
/// distinction -- both take an explicit file path and read straight
/// from `git blame`/`git log`, bypassing `RepoIndex` entirely.
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
    CSharp,
    Scala,
    Ruby,
    C,
    Swift,
    Php,
    Dart,
    Shell,
    ObjectiveC,
    R,
    Zig,
    Julia,
    Elm,
    OCaml,
    Crystal,
    Nim,
    D,
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
            "cs" => Language::CSharp,
            "scala" | "sc" => Language::Scala,
            "rb" => Language::Ruby,
            // `.h` is deliberately NOT mapped here: it's ambiguous
            // between C and C++ headers, and this port has no
            // C++-only-syntax sniffing to disambiguate — kept as
            // `Other` rather than guessed, same call already made (and
            // documented) for C++'s own extension set.
            "c" => Language::C,
            "swift" => Language::Swift,
            "php" => Language::Php,
            "dart" => Language::Dart,
            "sh" | "bash" | "zsh" => Language::Shell,
            "m" | "mm" => Language::ObjectiveC,
            // Capital `.R` is the dominant convention for R scripts;
            // lowercase `.r` also appears, so both are accepted.
            "r" | "R" => Language::R,
            "zig" => Language::Zig,
            "jl" => Language::Julia,
            "elm" => Language::Elm,
            "ml" | "mli" => Language::OCaml,
            "cr" => Language::Crystal,
            "nim" => Language::Nim,
            "d" => Language::D,
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
            Language::CSharp => "C#",
            Language::Scala => "Scala",
            Language::Ruby => "Ruby",
            Language::C => "C",
            Language::Swift => "Swift",
            Language::Php => "PHP",
            Language::Dart => "Dart",
            Language::Shell => "Shell",
            Language::ObjectiveC => "Objective-C",
            Language::R => "R",
            Language::Zig => "Zig",
            Language::Julia => "Julia",
            Language::Elm => "Elm",
            Language::OCaml => "OCaml",
            Language::Crystal => "Crystal",
            Language::Nim => "Nim",
            Language::D => "D",
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
    /// A mixin of concrete method implementations (PHP's `trait`) —
    /// distinct from `Trait` (an interface-like contract), since PHP's
    /// own `interface`/`trait` are two different constructs that this
    /// port's acceptance criteria call out separately.
    Mixin,
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
            SymbolKind::Mixin => "mixin",
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
    /// Maximum nesting depth of control-flow blocks (if/for/while/etc.)
    /// within the body (0 = no nested blocks at all). `0` for symbols
    /// with no body to analyze, same as `complexity`. Complements
    /// `complexity`: a function with 10 sequential ifs and one with the
    /// same 10 ifs nested inside each other score identically on
    /// cyclomatic complexity but read very differently.
    pub max_nesting_depth: usize,
    /// "Bumpy Road" count: number of distinct nested-block regions
    /// (leaf decision nodes nested >= 2 levels deep) within the body.
    /// `0` for symbols with no body to analyze, same as `complexity`.
    /// Complements `max_nesting_depth`: a function with three separate
    /// two-level-deep blocks reads worse than one with a single
    /// two-level-deep block, even at the same max nesting depth.
    pub bumpy_road_bumps: usize,
    /// Number of declared parameters. `0` for symbols without a
    /// parameter list.
    pub param_count: usize,
    /// Number of declared parameters whose type is a bare primitive
    /// (`i32`/`bool`/`String` and language equivalents) rather than a
    /// domain-specific type. `0` for languages without declared
    /// parameter types (dynamically-typed languages, or statically-typed
    /// ones this extraction isn't implemented for yet — see each
    /// language's own extraction logic in `repowise-parser`).
    pub primitive_param_count: usize,
    /// A hash of the body's whitespace-normalized text, used for
    /// best-effort duplicate-code detection. `None` when there's no body
    /// or the body is too short to be a meaningful duplicate signal.
    pub body_hash: Option<u64>,
    /// `if`/`while`/etc. conditions within the body chaining more than a
    /// documented threshold of boolean operators (`&&`/`||` and language
    /// equivalents). Empty for symbols with no body, and for languages
    /// this extraction isn't implemented for yet (see each language's
    /// own extraction logic in `repowise-parser`).
    pub complex_conditionals: Vec<ComplexConditionalRef>,
    /// Calls to a known I/O-shaped operation (file/network/database) found
    /// inside a loop body within the symbol, where hoisting the call above
    /// the loop is usually possible. Empty for symbols with no body, and
    /// for languages this extraction isn't implemented for yet (see each
    /// language's own extraction logic in `repowise-parser`) -- currently
    /// Rust, Python, and TypeScript/JavaScript, matching the scope
    /// LCOM4/`complex_conditional` already established.
    pub io_in_loop: Vec<IoInLoopRef>,
    /// String-append expressions (`+=`, `s = s + other`, `.push_str(..)`)
    /// accumulating onto a variable found inside a loop body -- quadratic
    /// string-building cost in most languages, since each append
    /// reallocates and copies the whole string so far. Empty for symbols
    /// with no body, and for languages this extraction isn't implemented
    /// for yet -- currently Rust, Python, and TypeScript/JavaScript,
    /// matching `io_in_loop`'s scope.
    pub string_concat_in_loop: Vec<StringConcatInLoopRef>,
    /// Calls recognized as constructing an expensive resource (an HTTP
    /// client, a connection/thread pool, etc. -- a small fixed per-language
    /// name table, heuristic, not type-aware) found inside a loop body
    /// within the symbol, where hoisting the construction above the loop
    /// is usually possible. Empty for symbols with no body, and for
    /// languages this extraction isn't implemented for yet -- currently
    /// Rust, Python, and TypeScript/JavaScript, matching `io_in_loop`'s
    /// scope.
    pub resource_construction_in_loop: Vec<ResourceConstructionInLoopRef>,
    /// Calls recognized as acquiring a mutex/lock (`.lock()`/`.acquire()`
    /// and language equivalents -- a small fixed per-language name table,
    /// heuristic, not type-aware) found inside a loop body within the
    /// symbol, where acquiring the lock once outside the loop is usually
    /// possible instead of repeated lock/unlock churn per iteration.
    /// Empty for symbols with no body, and for languages this extraction
    /// isn't implemented for yet -- currently Rust, Python, and
    /// TypeScript/JavaScript, matching `io_in_loop`'s scope.
    pub lock_in_loop: Vec<LockInLoopRef>,
    /// Calls recognized as inserting at index 0 of a list/vector
    /// (`.insert(0, ...)` and language equivalents -- a small fixed
    /// per-language pattern, heuristic, not type-aware) found inside a
    /// loop body within the symbol: O(n) per call (shifts every
    /// element), O(n^2) across the whole loop, versus appending and
    /// reversing once or using a deque. Empty for symbols with no body,
    /// and for languages this extraction isn't implemented for yet --
    /// currently Rust and Python only (this marker's own scope, unlike
    /// the other loop-body markers, doesn't include TypeScript/
    /// JavaScript).
    pub list_insert_zero_in_loop: Vec<ListInsertZeroInLoopRef>,
    /// Calls recognized as parsing a JSON payload (`serde_json::from_str`/
    /// `json.loads`/`JSON.parse` and language equivalents -- a small
    /// fixed per-language name table, heuristic, not type-aware) found
    /// inside a loop body within the symbol, where hoisting the parse
    /// call above the loop is usually possible if the payload doesn't
    /// change per iteration. Empty for symbols with no body, and for
    /// languages this extraction isn't implemented for yet -- currently
    /// Rust, Python, and TypeScript/JavaScript, matching `io_in_loop`'s
    /// scope.
    pub json_parse_in_loop: Vec<JsonParseInLoopRef>,
    /// Calls recognized as compiling a regex (`Regex::new`/`re.compile`/
    /// `new RegExp` and language equivalents -- a small fixed per-language
    /// name table, heuristic, not type-aware) found inside a loop body
    /// within the symbol, where hoisting the compile call above the loop
    /// is usually possible if the pattern doesn't change per iteration.
    /// Compiling a regex is orders of magnitude more expensive than using
    /// an already-compiled one. Empty for symbols with no body, and for
    /// languages this extraction isn't implemented for yet -- currently
    /// Rust, Python, and TypeScript/JavaScript, matching `io_in_loop`'s
    /// scope.
    pub regex_compile_in_loop: Vec<RegexCompileInLoopRef>,
}

/// A single flagged `if`/`while`/etc. condition: `line` points at the
/// condition itself, not the enclosing function, so `get_why`/dashboard/
/// wiki consumers can jump straight to the hard-to-read expression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexConditionalRef {
    pub line: usize,
    pub operator_count: usize,
}

/// A single call recognized as I/O-shaped (by a small fixed per-language
/// name table -- heuristic, not type-aware) found inside a loop body.
/// `line` points at the call itself, not the enclosing loop or function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoInLoopRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single string-append expression found inside a loop body. `line`
/// points at the append expression itself; `variable` is the name of the
/// string variable being appended onto.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringConcatInLoopRef {
    pub line: usize,
    pub variable: String,
}

/// A single call recognized as constructing an expensive resource (by a
/// small fixed per-language name table -- heuristic, not type-aware)
/// found inside a loop body. `line` points at the call itself, not the
/// enclosing loop or function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConstructionInLoopRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single call recognized as acquiring a mutex/lock (by a small fixed
/// per-language name table -- heuristic, not type-aware) found inside a
/// loop body. `line` points at the call itself, not the enclosing loop
/// or function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInLoopRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single call recognized as inserting at index 0 of a list/vector
/// (by a small fixed per-language pattern -- heuristic, not type-aware)
/// found inside a loop body. `line` points at the call itself, not the
/// enclosing loop or function; `variable` is the name of the list/vector
/// being inserted into.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListInsertZeroInLoopRef {
    pub line: usize,
    pub variable: String,
}

/// A single call recognized as parsing a JSON payload (by a small fixed
/// per-language name table -- heuristic, not type-aware) found inside a
/// loop body. `line` points at the call itself, not the enclosing loop
/// or function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonParseInLoopRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single call recognized as compiling a regex (by a small fixed
/// per-language name table -- heuristic, not type-aware) found inside a
/// loop body. `line` points at the call itself, not the enclosing loop
/// or function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexCompileInLoopRef {
    pub line: usize,
    pub callee_name: String,
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

/// A `self`/`this` field or property access (read or write; the
/// distinction doesn't matter for cohesion purposes, only that the
/// method touches the field) found inside a method body. Used to compute
/// LCOM4 (structural class cohesion) in `repowise-health`; not populated
/// for languages without field-access extraction (see `FileRecord`'s own
/// doc comment).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldAccessRef {
    /// The enclosing method's symbol id.
    pub method: SymbolId,
    pub field_name: String,
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
    /// Empty for languages/extractors that don't yet extract field
    /// accesses (see each language module's own extraction logic) —
    /// degrades LCOM4 scoring to "not enough data" for those files rather
    /// than failing.
    pub field_accesses: Vec<FieldAccessRef>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_extension_recognizes_every_structural_tier_language() {
        assert_eq!(Language::from_extension("m"), Language::ObjectiveC);
        assert_eq!(Language::from_extension("mm"), Language::ObjectiveC);
        assert_eq!(Language::from_extension("r"), Language::R);
        assert_eq!(Language::from_extension("R"), Language::R);
        assert_eq!(Language::from_extension("zig"), Language::Zig);
        assert_eq!(Language::from_extension("jl"), Language::Julia);
        assert_eq!(Language::from_extension("elm"), Language::Elm);
        assert_eq!(Language::from_extension("ml"), Language::OCaml);
        assert_eq!(Language::from_extension("mli"), Language::OCaml);
        assert_eq!(Language::from_extension("cr"), Language::Crystal);
        assert_eq!(Language::from_extension("nim"), Language::Nim);
        assert_eq!(Language::from_extension("d"), Language::D);
    }

    #[test]
    fn from_extension_falls_back_to_other_for_unknown_extensions() {
        assert_eq!(Language::from_extension("xyz"), Language::Other);
    }

    #[test]
    fn structural_tier_languages_have_distinct_labels() {
        let labels = [
            Language::ObjectiveC,
            Language::R,
            Language::Zig,
            Language::Julia,
            Language::Elm,
            Language::OCaml,
            Language::Crystal,
            Language::Nim,
            Language::D,
        ]
        .map(|l| l.label());
        let mut sorted = labels.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len());
    }
}
