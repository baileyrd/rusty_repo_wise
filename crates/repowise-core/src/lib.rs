//! Shared data model and index persistence for repowise.
//!
//! This crate defines the on-disk index format and the language-agnostic
//! structures produced by `repowise-parser` and consumed by `repowise-graph`.

pub mod coverage;
pub mod deps;
pub mod docker;
pub mod graphql;
pub mod openapi;
pub mod org_signals;
pub mod portable;
pub mod protobuf;
pub mod sql;
pub mod terraform;
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
    /// Luau (issue #341), Roblox's Lua dialect -- via the community
    /// `tree-sitter-luau` grammar, the same "Full tier" extraction depth
    /// as every language above it, not the shallower "Partial" tier
    /// upstream repowise defines for it (see `repowise_parser::luau`'s
    /// module doc for why that tier concept isn't needed here).
    Luau,
    ObjectiveC,
    R,
    Zig,
    Julia,
    Elm,
    OCaml,
    Crystal,
    Nim,
    D,
    /// A Dockerfile (issue #318, the prototype for #68's config/data
    /// tier) -- recognized by filename in `discover_files`, not by
    /// extension (see that module), and given a real zero-symbol
    /// `FileRecord` the same way the Structural tier is: no grammar to
    /// extract symbols from, but visible to git-history-derived views.
    /// Its actual content -- build stages, `COPY --from` edges -- is a
    /// parallel model ([`docker::DockerStage`]) computed separately by
    /// `repowise_parser::collect_docker_stages`, not part of
    /// `FileRecord` at all.
    Dockerfile,
    /// The "Lightweight" tier (issue #69): a file-level import graph via
    /// regex, and nothing else -- no symbols, no calls, no complexity.
    /// Deliberately shallower than every other supported language, all
    /// of which get full tree-sitter AST extraction. Each `ImportRef`
    /// this tier produces is left unresolved by design (see
    /// `repowise_parser::lightweight`'s module doc for why) -- the same
    /// choice already made for Swift's/Dart's package imports.
    Elixir,
    Clojure,
    Haskell,
    /// Lean 4, not Lean 3 -- the two have incompatible syntax and this
    /// port only recognizes Lean 4's `import Foo.Bar` form.
    Lean,
    Erlang,
    FSharp,
    /// SQL, including dbt models (issue #317, the buildable follow-up to
    /// #67's design decision): recognized here and given the same
    /// "Structural tier" bare zero-symbol `FileRecord` treatment as
    /// Dockerfile above, so `.sql` files are visible in
    /// `repowise overview`'s per-language counts and git-history views.
    /// Its actual content -- tables/views/functions/procedures, dbt
    /// `ref()`/`source()` lineage -- is a parallel model
    /// ([`sql::SqlObject`]/[`sql::LineageEdge`]) computed separately by
    /// `repowise_sql::collect_sql`, not part of `FileRecord` at all.
    Sql,
    /// Protobuf (issue #324, the buildable follow-up to #319's design
    /// decision): recognized here and given the same "Structural tier"
    /// bare zero-symbol `FileRecord` treatment as Dockerfile/SQL above,
    /// so `.proto` files are visible in `repowise overview`'s
    /// per-language counts and git-history views -- unlike OpenAPI
    /// (#323), `.proto` has an unambiguous extension, so there's no
    /// content-sniffing problem to avoid here. Its actual content --
    /// messages/services/RPCs -- is a parallel model
    /// ([`protobuf::ProtoObject`]) computed separately by
    /// `repowise_protobuf::collect_protobuf`, not part of `FileRecord`
    /// at all.
    Proto,
    /// GraphQL SDL (issue #325, the buildable follow-up to #319's
    /// design decision): same "Structural tier" treatment as Protobuf
    /// above -- `.graphql`/`.gql` also has an unambiguous extension.
    /// Its actual content -- types/queries/mutations/subscriptions --
    /// is a parallel model ([`graphql::GraphQlObject`]) computed
    /// separately by `repowise_graphql::collect_graphql`, not part of
    /// `FileRecord` at all.
    GraphQl,
    /// Terraform (issue #326, the buildable follow-up to #319's design
    /// decision): same "Structural tier" treatment as Protobuf/GraphQL
    /// above -- `.tf` also has an unambiguous extension. Its actual
    /// content -- `resource`/`module` blocks -- is a parallel model
    /// ([`terraform::TerraformResource`]/[`terraform::TerraformModule`])
    /// computed separately by `repowise_terraform::collect_terraform`,
    /// not part of `FileRecord` at all. Unlike the other three schema
    /// formats' single object type, these are deliberately two separate
    /// types: a `resource` block isn't a schema with fields the way a
    /// SQL table/OpenAPI schema/protobuf message/GraphQL type is -- it's
    /// a named instance whose shape Terraform can't know statically
    /// (that requires the provider plugin).
    Terraform,
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
            // `.lua` is deliberately NOT mapped here: plain Lua and Luau
            // share the `.lua` extension in the wild, but this port only
            // has a Luau grammar (which rejects some plain-Lua-5.1-only
            // syntax) -- `.luau` is Roblox tooling's own unambiguous
            // convention for "this file is Luau", so only that extension
            // is recognized, the same "don't guess past an ambiguous
            // extension" call already made for C++'s `.h`.
            "luau" => Language::Luau,
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
            // The bare-filename case (`Dockerfile`, `Dockerfile.dev`) is
            // handled in `discover_files` before extension lookup even
            // runs; this arm only covers the `*.dockerfile` suffix
            // convention (e.g. `backend.dockerfile`).
            "dockerfile" => Language::Dockerfile,
            "ex" | "exs" => Language::Elixir,
            "clj" | "cljs" | "cljc" => Language::Clojure,
            "hs" | "lhs" => Language::Haskell,
            "lean" => Language::Lean,
            "erl" | "hrl" => Language::Erlang,
            "fs" | "fsi" | "fsx" => Language::FSharp,
            "sql" => Language::Sql,
            "proto" => Language::Proto,
            "graphql" | "gql" => Language::GraphQl,
            "tf" => Language::Terraform,
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
            Language::Luau => "Luau",
            Language::ObjectiveC => "Objective-C",
            Language::R => "R",
            Language::Zig => "Zig",
            Language::Julia => "Julia",
            Language::Elm => "Elm",
            Language::OCaml => "OCaml",
            Language::Crystal => "Crystal",
            Language::Nim => "Nim",
            Language::D => "D",
            Language::Dockerfile => "Dockerfile",
            Language::Elixir => "Elixir",
            Language::Clojure => "Clojure",
            Language::Haskell => "Haskell",
            Language::Lean => "Lean 4",
            Language::Erlang => "Erlang",
            Language::FSharp => "F#",
            Language::Sql => "SQL",
            Language::Proto => "Protobuf",
            Language::GraphQl => "GraphQL",
            Language::Terraform => "Terraform",
            Language::Other => "Other",
        }
    }
}

/// Kind of a definition site extracted from source.
///
/// `Default` is derived **only in test builds** (`cfg_attr(test, ...)`),
/// purely so test fixtures can use `..Default::default()` on [`Symbol`]
/// without spelling out its twenty-odd health-marker fields. There is no
/// meaningful "default kind" for real extracted code, so production
/// builds deliberately don't get one to reach for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(Default))]
pub enum SymbolKind {
    #[cfg_attr(test, default)]
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
///
/// `Default` is test-build-only — see [`SymbolKind`] for why.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(Default))]
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
    /// isn't implemented for yet -- currently Rust and Python, and
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
    /// I/O-shaped calls (same small fixed per-language name table as
    /// `io_in_loop`) found at loop-nesting depth 2 or deeper within the
    /// symbol -- worse than a single-loop `io_in_loop` hit, since it's
    /// potentially O(n^2) (or deeper) I/O calls rather than O(n). A call
    /// reported here is also reported in `io_in_loop` (this is a
    /// depth-2+ subset, not a separate detection pass), so the two
    /// markers double-count on purpose -- the outer one measures "any
    /// loop-body I/O", the inner one measures the specifically worse
    /// nested case. Empty for symbols with no body, and for languages
    /// this extraction isn't implemented for yet -- currently Rust,
    /// Python, and TypeScript/JavaScript, matching `io_in_loop`'s scope.
    pub nested_loop_with_io: Vec<NestedLoopWithIoRef>,
    /// Inner loops whose iterated collection is the same as (or a
    /// trivial derivation of) an enclosing loop's -- the classic
    /// accidental all-pairs O(n^2) scan (`for x in items { for y in
    /// items { .. } }`), usually replaceable with a set/map lookup.
    /// Compares the two loops' *iterable expressions*, unlike
    /// `nested_loop_with_io`, which only cares about nesting depth and
    /// what's called inside. Empty for symbols with no body, and for
    /// languages this extraction isn't implemented for yet -- currently
    /// Rust, Python, and TypeScript/JavaScript, matching `io_in_loop`'s
    /// scope.
    pub nested_loop_quadratic: Vec<NestedLoopQuadraticRef>,
    /// Awaited async calls found inside a loop body within the symbol,
    /// where each iteration blocks on the previous one instead of the
    /// whole batch running concurrently (`Promise.all`/`join_all`/
    /// `asyncio.gather`). Awaits *of* those concurrency combinators are
    /// themselves excluded: awaiting one inside a loop is the chunked-
    /// concurrency shape, not the serial one this marker is after. Empty
    /// for symbols with no body, and for languages this extraction isn't
    /// implemented for yet -- currently Rust, Python, and TypeScript/
    /// JavaScript, matching `io_in_loop`'s scope.
    pub serial_await_in_loop: Vec<SerialAwaitInLoopRef>,
    /// `pandas.concat` calls found inside a loop body within the symbol,
    /// accumulating rows one at a time instead of collecting them and
    /// concatenating once after the loop. Each call reallocates and
    /// copies the whole growing DataFrame, making the loop quadratic in
    /// the number of rows. Python-only (pandas has no equivalent in this
    /// port's other supported languages), and empty for symbols with no
    /// body.
    pub pd_concat_in_loop: Vec<PdConcatInLoopRef>,
    /// Blocking, synchronous calls (`std::thread::sleep`, `time.sleep`,
    /// `requests.get`, blocking `std::fs`/`open`) found inside an
    /// `async fn`/`async def` body. A blocking call on an async
    /// executor's worker thread stalls the whole reactor, degrading
    /// every other task sharing that thread. Unlike the loop-body
    /// markers, the context here is the enclosing *function* being
    /// async, not an enclosing loop. Empty for non-async functions, for
    /// symbols with no body, and for languages this extraction isn't
    /// implemented for yet -- currently Rust and Python, the two the
    /// issue scoped it to.
    pub blocking_sync_in_async: Vec<BlockingSyncInAsyncRef>,
    /// I/O-shaped calls (the same table as `io_in_loop`) made while a
    /// mutex/lock is held. I/O under a lock serializes every other
    /// thread waiting on it behind however long the I/O takes, turning
    /// an in-memory critical section into a throughput bottleneck.
    /// Empty for symbols with no body, and for languages this extraction
    /// isn't implemented for yet -- currently Rust and Python, the two
    /// the issue scoped it to.
    pub blocking_io_under_lock: Vec<BlockingIoUnderLockRef>,
    /// `.reduce(..)` callbacks that build their result with array spread
    /// (`(acc, x) => [...acc, x]`) instead of mutating and returning the
    /// accumulator. The spread copies the entire accumulator on every
    /// step, turning a linear fold into a quadratic one. TypeScript/
    /// JavaScript only -- this is specific to the JS array method, with
    /// no equivalent in this port's other languages.
    pub array_spread_in_reduce: Vec<ArraySpreadInReduceRef>,
    /// SQL query strings that list several comma-joined tables without
    /// enough join predicates to connect them -- an accidental cartesian
    /// product returning `n * m` rows. A text-level scan of string
    /// literals, not a real SQL parse. Empty for symbols with no body,
    /// and for languages this extraction isn't implemented for yet --
    /// currently Rust, Python, and TypeScript/JavaScript.
    pub sql_cartesian_join: Vec<SqlCartesianJoinRef>,
    /// `defer` statements inside a loop body. Go defers run at the
    /// *enclosing function's* return, not at the end of the iteration
    /// that created them, so a `defer f.Close()` in a loop holds every
    /// file open until the whole function exits. Go only -- no other
    /// language in this port has a defer-to-function-exit construct.
    pub defer_in_loop: Vec<DeferInLoopRef>,
    /// `go` statements inside a loop body with no visible concurrency
    /// bound, spawning one goroutine per iteration. Go only -- no other
    /// language in this port launches concurrency with a bare keyword.
    pub goroutine_in_unbounded_loop: Vec<GoroutineInUnboundedLoopRef>,
    /// `x in list`-shaped membership tests inside a loop body, where the
    /// tested collection is known to be a list/array rather than a
    /// set/map. Each check is O(n), so running one per iteration makes
    /// the loop quadratic where a set lookup would keep it linear.
    /// Rust/Python/TypeScript+JavaScript only, and only where the
    /// collection's kind is locally evident -- see
    /// `repowise_parser::metrics::list_membership_tests_in_loops`.
    pub membership_test_in_loop: Vec<MembershipTestInLoopRef>,
    /// Synchronous, blocking I/O-shaped calls found anywhere in this
    /// function's body -- not just inside a loop, unlike `io_in_loop`.
    /// On its own this is not a finding: a blocking read in a rarely-run
    /// setup path is fine. `repowise-health` only reports these when the
    /// containing file is *also* a git hotspot, which is what makes the
    /// combination `hot_path_sync_io`. Rust/Python/TypeScript+JavaScript
    /// only, the same scope as `io_in_loop`'s callee table.
    pub sync_io_calls: Vec<SyncIoCallRef>,
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

/// A single I/O-shaped call (by the same small fixed per-language name
/// table as `IoInLoopRef` -- heuristic, not type-aware) found at
/// loop-nesting depth 2 or deeper. `line` points at the call itself, not
/// the enclosing loops or function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NestedLoopWithIoRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single inner loop iterating the same collection as an enclosing
/// loop. `line` points at the inner loop itself, not the outer one or
/// the function; `iterable` is the shared collection's name, normalized
/// past trivial derivations (`&items`/`items.iter()` all report
/// `items`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NestedLoopQuadraticRef {
    pub line: usize,
    pub iterable: String,
}

/// A single awaited async call found inside a loop body. `line` points
/// at the await itself, not the enclosing loop or function;
/// `callee_name` is the awaited call's callee.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialAwaitInLoopRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single `pandas.concat` call found inside a loop body. `line` points
/// at the call itself, not the enclosing loop or function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdConcatInLoopRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single blocking synchronous call found inside an async function
/// body. `line` points at the call itself, not the enclosing function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockingSyncInAsyncRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single I/O-shaped call found while a lock is held. `line` points at
/// the call itself, not the lock acquisition or the enclosing function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockingIoUnderLockRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single `.reduce(..)` callback spreading its accumulator into a new
/// array. `line` points at the `reduce` call; `accumulator` is the
/// callback's first parameter name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArraySpreadInReduceRef {
    pub line: usize,
    pub accumulator: String,
}

/// A single SQL string literal that looks like an accidental cartesian
/// join. `line` points at the string literal; `tables` lists the
/// comma-joined table names that appeared unconnected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlCartesianJoinRef {
    pub line: usize,
    pub tables: String,
}

/// A single `defer` statement found inside a loop body. `line` points at
/// the `defer` itself; `callee_name` is the deferred call's function or
/// method name (`Close`, `Unlock`, ...), which is what makes the finding
/// actionable -- the resource being held is named right there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeferInLoopRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single `go` statement launched inside a loop body that has no
/// visible concurrency bound. `line` points at the `go`; `callee_name`
/// is the launched call's name, or `func literal` for the inline
/// `go func() {...}()` form, which has no name to report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoroutineInUnboundedLoopRef {
    pub line: usize,
    pub callee_name: String,
}

/// A single membership test against a list inside a loop body. `line`
/// points at the test; `collection` is the tested variable's name, or
/// `<list literal>` when the list is written inline at the test site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipTestInLoopRef {
    pub line: usize,
    pub collection: String,
}

/// A single synchronous I/O-shaped call in a function body. `line`
/// points at the call; `callee_name` is the recognized I/O callee.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncIoCallRef {
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
    /// The commit `HEAD` pointed at when this index was built, as a
    /// 12-character SHA prefix — the anchor for "is this index still
    /// describing the current tree?".
    ///
    /// `None` means *unknown*, not *matching*: the repo has no git, no
    /// commits, or the index predates this field. Consumers must report
    /// unknown as unknown; treating it as "up to date" would make every
    /// index built before this field existed claim to be current
    /// forever.
    ///
    /// `#[serde(default)]` so an index written before this field was
    /// added still loads instead of failing to parse — a re-index is a
    /// reasonable thing to ask for, a hard load error on an old index
    /// is not.
    #[serde(default)]
    pub indexed_commit: Option<String>,
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
