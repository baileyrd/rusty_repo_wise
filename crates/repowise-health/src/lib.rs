//! Deterministic, rule-based code-health scoring — no LLM/ML involved.
//!
//! This implements a focused subset of repowise's "25 deterministic
//! markers": long functions, high cyclomatic complexity, oversized
//! parameter lists, god classes (too many methods), duplicate code
//! (identical function/method bodies), near-duplicate code (Rabin-Karp
//! rolling-hash overlap — see the `near_duplicate` module doc comment),
//! possibly-dead code (zero resolved callers), low cohesion (LCOM4 —
//! see the `lcom4` module doc comment for its Rust/Python/TS+JS-only
//! scope), nested complexity (max control-flow nesting depth — see
//! `repowise_core::Symbol::max_nesting_depth`), a "bumpy road" (count of
//! distinct nested-block regions — see
//! `repowise_core::Symbol::bumpy_road_bumps`), and complex conditionals
//! (per-condition boolean-operator chains, Rust/Python/TS+JS-only — see
//! `repowise_core::Symbol::complex_conditionals`), and primitive obsession
//! (parameter lists leaning on bare primitives instead of domain types,
//! Rust/TypeScript-only since it needs declared parameter types — see
//! `repowise_core::Symbol::primitive_param_count`), and nineteen of
//! repowise's Performance-signal cluster (issue #72): I/O-shaped calls
//! found inside a loop body (`io_in_loop`, Rust/Python/TS+JS-only — see
//! `repowise_core::Symbol::io_in_loop`), string concatenation
//! accumulating inside a loop body (`string_concat_in_loop`, same scope
//! — see `repowise_core::Symbol::string_concat_in_loop`), expensive-resource
//! construction inside a loop body (`resource_construction_in_loop`, same
//! scope — see `repowise_core::Symbol::resource_construction_in_loop`),
//! lock acquisition inside a loop body (`lock_in_loop`, same scope — see
//! `repowise_core::Symbol::lock_in_loop`), index-0 list/vector
//! inserts inside a loop body (`list_insert_zero_in_loop`, Rust/Python
//! only -- see `repowise_core::Symbol::list_insert_zero_in_loop`), and
//! JSON-parsing calls inside a loop body (`json_parse_in_loop`, same
//! scope as `io_in_loop` -- see
//! `repowise_core::Symbol::json_parse_in_loop`), and regex-compilation
//! calls inside a loop body (`regex_compile_in_loop`, same scope as
//! `io_in_loop` -- see `repowise_core::Symbol::regex_compile_in_loop`),
//! and I/O calls found at loop-nesting depth 2+ (`nested_loop_with_io`,
//! same scope as `io_in_loop` -- a depth-2+ subset of it, not a separate
//! detection pass, so a flagged call is reported under both markers --
//! see `repowise_core::Symbol::nested_loop_with_io`), and inner loops
//! iterating the same collection as an enclosing loop
//! (`nested_loop_quadratic`, same scope as `io_in_loop` -- see
//! `repowise_core::Symbol::nested_loop_quadratic`), and awaited async
//! calls inside a loop body running serially instead of concurrently
//! (`serial_await_in_loop`, same scope as `io_in_loop` -- see
//! `repowise_core::Symbol::serial_await_in_loop`), and `pandas.concat`
//! calls inside a loop body (`pd_concat_in_loop`, Python-only -- see
//! `repowise_core::Symbol::pd_concat_in_loop`), and blocking synchronous
//! calls inside an async function body (`blocking_sync_in_async`,
//! Rust/Python-only, and the one marker in this cluster whose context is
//! the enclosing *function* rather than an enclosing loop -- see
//! `repowise_core::Symbol::blocking_sync_in_async`), and I/O-shaped
//! calls made while a lock is held (`blocking_io_under_lock`,
//! Rust/Python-only -- see
//! `repowise_core::Symbol::blocking_io_under_lock`), and `.reduce(..)`
//! callbacks spreading their accumulator into a new array
//! (`array_spread_in_reduce`, TS/JS-only -- see
//! `repowise_core::Symbol::array_spread_in_reduce`), and SQL strings
//! that look like accidental cartesian joins (`sql_cartesian_join`,
//! Rust/Python/TS+JS -- see
//! `repowise_core::Symbol::sql_cartesian_join`), and `defer` statements
//! inside a loop body (`defer_in_loop`, Go-only, since no other language
//! here has a defer-to-function-exit construct -- see
//! `repowise_core::Symbol::defer_in_loop`), and `go` statements
//! launched inside a loop body with no visible concurrency bound
//! (`goroutine_in_unbounded_loop`, Go-only -- see
//! `repowise_core::Symbol::goroutine_in_unbounded_loop`), and
//! membership tests against a list inside a loop body
//! (`membership_test_in_loop`, Rust/Python/TS+JS-only, and only where a
//! local binding makes the collection's kind evident -- see
//! `repowise_core::Symbol::membership_test_in_loop`), and synchronous
//! I/O calls made from a function whose file git analytics flags as a
//! hotspot (`hot_path_sync_io`, Rust/Python/TS+JS-only, and the only
//! marker here built from an empirical signal as well as a structural
//! one -- see `analyze_with_hotspots`).
//! Git-history-based markers (churn, hotspots, bug-fix history) aren't
//! implemented yet — that needs the git-analytics layer, which is a
//! separate phase.
//!
//! Every marker here is a plain threshold over data `repowise-parser`/
//! `repowise-graph` already computed; nothing is inferred or guessed.
//! The one exception is `near_duplicate`, which re-reads source text
//! fresh from disk since `Symbol` doesn't carry raw body text — see its
//! own module doc comment for why that's still consistent with this
//! crate's usual "no I/O" shape rather than a quiet exception to it.

mod dead_code;
mod lcom4;
mod near_duplicate;

pub use dead_code::{find_dead_code, DeadCodeCandidate, DeadCodeConfidence};
pub use lcom4::{find_low_cohesion, LowCohesionCandidate, LOW_COHESION_MIN_COMPONENTS};
pub use near_duplicate::{find_near_duplicates, NearDuplicateCandidate};

use repowise_core::{Language, RepoIndex, Symbol, SymbolKind};
use repowise_graph::RepoGraph;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// A function/method longer than this (in lines) is flagged.
pub const LONG_FUNCTION_LINES: usize = 50;
/// A function/method with cyclomatic complexity above this is flagged.
pub const HIGH_COMPLEXITY: usize = 10;
/// A function/method with more than this many parameters is flagged.
pub const TOO_MANY_PARAMS: usize = 6;
/// A struct/class with more than this many methods is flagged ("god class").
pub const GOD_CLASS_METHODS: usize = 15;
/// A function/method with control-flow nested deeper than this is flagged.
/// E.g. an `if` inside a `for` inside an `if` is depth 3.
pub const HIGH_NESTING_DEPTH: usize = 4;
/// A function/method with at least this many "bumpy road" nested-block
/// regions (see `repowise_core::Symbol::bumpy_road_bumps`) is flagged.
pub const BUMPY_ROAD_MIN_BUMPS: usize = 3;
/// A function/method with at least this many bare-primitive-typed
/// parameters (see `repowise_core::Symbol::primitive_param_count`) is
/// flagged.
pub const PRIMITIVE_OBSESSION_MIN_COUNT: usize = 3;

const MAX_SCORE: f64 = 10.0;

fn default_long_function() -> f64 {
    0.5
}
fn default_high_complexity() -> f64 {
    1.0
}
fn default_too_many_params() -> f64 {
    0.3
}
fn default_god_class() -> f64 {
    1.5
}
fn default_duplicate_code() -> f64 {
    0.5
}
fn default_dead_code() -> f64 {
    0.2
}
fn default_low_cohesion() -> f64 {
    1.0
}
// Weaker signal than an exact-hash `DuplicateCode` match (a heuristic
// overlap ratio, not a byte-for-byte match), so it's penalized less.
fn default_near_duplicate() -> f64 {
    0.3
}
// Same weight as `HighComplexity`: both are cheap AST-derived structural
// signals of the same rough severity, just measuring different things
// (branch count vs. nesting depth).
fn default_nested_complexity() -> f64 {
    1.0
}
// Lighter than `NestedComplexity`: a complementary signal on the same
// underlying data (scattered nesting vs. a single deep point), not an
// independent problem worth double-weighting.
fn default_bumpy_road() -> f64 {
    0.5
}
// A function can rack up multiple flagged conditions at once; a
// per-occurrence weight lighter than the whole-function markers above
// avoids one messy function alone tanking its score.
fn default_complex_conditional() -> f64 {
    0.3
}
// Same weight as `TooManyParameters`/`ComplexConditional`: another
// parameter-list-shaped structural-complexity signal, not a central-logic
// problem worth a heavier penalty.
fn default_primitive_obsession() -> f64 {
    0.3
}
// A function can rack up multiple flagged calls at once (issue #177); a
// per-occurrence weight, same as `ComplexConditional`'s -- a real
// performance risk but not automatically worse than the whole-function
// structural markers above.
fn default_io_in_loop() -> f64 {
    0.3
}
// Same per-occurrence weight as `IoInLoop` (issue #178): another
// loop-body performance pattern, same rough severity.
fn default_string_concat_in_loop() -> f64 {
    0.3
}
// Same per-occurrence weight as `IoInLoop`/`StringConcatInLoop`
// (issue #179): another loop-body performance pattern, same rough
// severity.
fn default_resource_construction_in_loop() -> f64 {
    0.3
}
// Same per-occurrence weight as the other loop-body markers (issue
// #180): same rough severity.
fn default_lock_in_loop() -> f64 {
    0.3
}
// Same per-occurrence weight as the other loop-body markers (issue
// #191): same rough severity.
fn default_list_insert_zero_in_loop() -> f64 {
    0.3
}
// Same per-occurrence weight as the other loop-body markers (issue
// #193): same rough severity.
fn default_json_parse_in_loop() -> f64 {
    0.3
}
// Same per-occurrence weight as the other loop-body markers (issue
// #188): same rough severity.
fn default_regex_compile_in_loop() -> f64 {
    0.3
}
// Heavier than the other loop-body markers' flat 0.3 (issue #183): a
// loop-nesting depth of 2+ makes the I/O call potentially O(n^2) (or
// deeper), not just O(n), so it earns a heavier per-occurrence penalty
// even though every occurrence here also already counts once toward
// `io_in_loop`'s own penalty.
fn default_nested_loop_with_io() -> f64 {
    0.6
}
// Same weight as `NestedLoopWithIo` (issue #187): both flag a
// quadratic-complexity *shape* rather than a single expensive call, so
// both sit a tier above the flat 0.3 per-occurrence loop-body markers.
// Unlike that one, this marker double-counts nothing -- an inner loop
// iterating its outer loop's collection isn't reported by any other
// marker.
fn default_nested_loop_quadratic() -> f64 {
    0.6
}
// Same per-occurrence weight as the other loop-body markers (issue
// #181): a single serialized await is a real cost but, unlike the
// quadratic-*shape* markers above, one occurrence is one extra
// round-trip rather than an order-of-magnitude blowup.
fn default_serial_await_in_loop() -> f64 {
    0.3
}
// Same weight as the quadratic-*shape* markers rather than the flat 0.3
// per-occurrence tier (issue #192): a `pandas.concat` inside a loop
// copies the whole growing DataFrame every iteration, so a single
// occurrence makes the enclosing loop quadratic in the number of rows --
// an order-of-magnitude blowup, not one extra unit of work.
fn default_pd_concat_in_loop() -> f64 {
    0.6
}
// Elevated above the flat 0.3 per-occurrence tier (issue #184) for a
// different reason than the quadratic-shape markers: a blocking call on
// an async executor's worker thread stalls the whole reactor, so the
// cost lands on every *other* task sharing that thread rather than only
// on the function containing it.
fn default_blocking_sync_in_async() -> f64 {
    0.6
}
// Same elevated weight as `BlockingSyncInAsync` (issue #185), for the
// same reason: I/O under a lock serializes every *other* thread waiting
// on it behind however long the I/O takes, so the cost lands outside the
// function containing the call.
fn default_blocking_io_under_lock() -> f64 {
    0.6
}
// Same weight as the other quadratic-*shape* markers (issue #194): a
// single spread-accumulator reduce turns a linear fold into a quadratic
// one, an order-of-magnitude blowup rather than one extra unit of work.
fn default_array_spread_in_reduce() -> f64 {
    0.6
}
// Same weight as the other quadratic-*shape* markers (issue #195): a
// missing join predicate turns an `n`-row and `m`-row query into an
// `n * m`-row one. Arguably the most severe marker in the cluster --
// it's a correctness bug as well as a performance one -- but it's also
// the most heuristic (a text scan, not a SQL parse), so it sits at the
// same tier rather than above it.
fn default_sql_cartesian_join() -> f64 {
    0.6
}
// The heavier tier (issue #189) for the same reason as
// `blocking_io_under_lock`: the cost doesn't land on the flagged line.
// One `defer` inside a loop is a single occurrence but leaks one
// resource *per iteration*, so the damage scales with the loop's trip
// count rather than with how many times the marker fires -- counting it
// linearly like `lock_in_loop` would understate it badly.
fn default_defer_in_loop() -> f64 {
    0.6
}
// The heavier tier again (issue #190), for the same reason as
// `defer_in_loop`: one `go` statement in a loop fires the marker once
// but spawns one goroutine per iteration, so the cost scales with the
// loop's trip count. Unlike a leaked file handle, an unbounded goroutine
// fan-out also lands on whatever the goroutines call -- exhausting
// memory locally or overwhelming a downstream service.
fn default_goroutine_in_unbounded_loop() -> f64 {
    0.6
}
// The quadratic-*shape* tier (issue #182), same as
// `nested_loop_quadratic`: an O(n) membership test run once per
// iteration of an m-element loop is an O(n * m) shape written as if it
// were linear. The marker is already low-recall by construction (it only
// fires when a local binding proves the collection is a list), so the
// occurrences it does report are worth the heavier weight.
fn default_membership_test_in_loop() -> f64 {
    0.6
}
// The per-occurrence tier (issue #186), same as `io_in_loop`: an
// individual blocking call is a real cost but a bounded one, and unlike
// the quadratic-shape markers it doesn't get worse with input size. The
// precision here comes from the hotspot gate rather than from a heavier
// weight -- the marker only fires at all where churn history says the
// code actually runs and changes.
fn default_hot_path_sync_io() -> f64 {
    0.3
}

/// Per-marker scoring weights — the abstraction layer this crate's
/// penalties live behind. `Default` matches the hand-picked values this
/// crate always used (nothing changes for a caller that doesn't build
/// its own `HealthWeights`); a caller can instead load one from a
/// partial TOML file (`from_toml_str`, one key per `FindingKind`'s
/// snake_case field name — an omitted key keeps its documented default)
/// and pass it to `analyze_with_weights`.
///
/// This is a precursor for issue #62 (ML-calibrated health-score
/// weights), not the calibration itself: an actual calibrated weight
/// set still needs a labeled defect corpus and a training pipeline this
/// port doesn't have. What this type unblocks is having *somewhere* to
/// plug calibrated numbers into once they exist, without touching any
/// scoring logic — today every caller still gets the same fixed
/// penalties as before, just no longer hardcoded as unreachable consts.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct HealthWeights {
    #[serde(default = "default_long_function")]
    pub long_function: f64,
    #[serde(default = "default_high_complexity")]
    pub high_complexity: f64,
    #[serde(default = "default_too_many_params")]
    pub too_many_params: f64,
    #[serde(default = "default_god_class")]
    pub god_class: f64,
    #[serde(default = "default_duplicate_code")]
    pub duplicate_code: f64,
    #[serde(default = "default_near_duplicate")]
    pub near_duplicate_code: f64,
    #[serde(default = "default_dead_code")]
    pub possibly_dead_code: f64,
    #[serde(default = "default_low_cohesion")]
    pub low_cohesion: f64,
    #[serde(default = "default_nested_complexity")]
    pub nested_complexity: f64,
    #[serde(default = "default_bumpy_road")]
    pub bumpy_road: f64,
    #[serde(default = "default_complex_conditional")]
    pub complex_conditional: f64,
    #[serde(default = "default_primitive_obsession")]
    pub primitive_obsession: f64,
    #[serde(default = "default_io_in_loop")]
    pub io_in_loop: f64,
    #[serde(default = "default_string_concat_in_loop")]
    pub string_concat_in_loop: f64,
    #[serde(default = "default_resource_construction_in_loop")]
    pub resource_construction_in_loop: f64,
    #[serde(default = "default_lock_in_loop")]
    pub lock_in_loop: f64,
    #[serde(default = "default_list_insert_zero_in_loop")]
    pub list_insert_zero_in_loop: f64,
    #[serde(default = "default_json_parse_in_loop")]
    pub json_parse_in_loop: f64,
    #[serde(default = "default_regex_compile_in_loop")]
    pub regex_compile_in_loop: f64,
    #[serde(default = "default_nested_loop_with_io")]
    pub nested_loop_with_io: f64,
    #[serde(default = "default_nested_loop_quadratic")]
    pub nested_loop_quadratic: f64,
    #[serde(default = "default_serial_await_in_loop")]
    pub serial_await_in_loop: f64,
    #[serde(default = "default_pd_concat_in_loop")]
    pub pd_concat_in_loop: f64,
    #[serde(default = "default_blocking_sync_in_async")]
    pub blocking_sync_in_async: f64,
    #[serde(default = "default_blocking_io_under_lock")]
    pub blocking_io_under_lock: f64,
    #[serde(default = "default_array_spread_in_reduce")]
    pub array_spread_in_reduce: f64,
    #[serde(default = "default_sql_cartesian_join")]
    pub sql_cartesian_join: f64,
    #[serde(default = "default_defer_in_loop")]
    pub defer_in_loop: f64,
    #[serde(default = "default_goroutine_in_unbounded_loop")]
    pub goroutine_in_unbounded_loop: f64,
    #[serde(default = "default_membership_test_in_loop")]
    pub membership_test_in_loop: f64,
    #[serde(default = "default_hot_path_sync_io")]
    pub hot_path_sync_io: f64,
}

impl Default for HealthWeights {
    fn default() -> Self {
        HealthWeights {
            long_function: default_long_function(),
            high_complexity: default_high_complexity(),
            too_many_params: default_too_many_params(),
            god_class: default_god_class(),
            duplicate_code: default_duplicate_code(),
            near_duplicate_code: default_near_duplicate(),
            possibly_dead_code: default_dead_code(),
            low_cohesion: default_low_cohesion(),
            nested_complexity: default_nested_complexity(),
            bumpy_road: default_bumpy_road(),
            complex_conditional: default_complex_conditional(),
            primitive_obsession: default_primitive_obsession(),
            io_in_loop: default_io_in_loop(),
            string_concat_in_loop: default_string_concat_in_loop(),
            resource_construction_in_loop: default_resource_construction_in_loop(),
            lock_in_loop: default_lock_in_loop(),
            list_insert_zero_in_loop: default_list_insert_zero_in_loop(),
            json_parse_in_loop: default_json_parse_in_loop(),
            regex_compile_in_loop: default_regex_compile_in_loop(),
            nested_loop_with_io: default_nested_loop_with_io(),
            nested_loop_quadratic: default_nested_loop_quadratic(),
            serial_await_in_loop: default_serial_await_in_loop(),
            pd_concat_in_loop: default_pd_concat_in_loop(),
            blocking_sync_in_async: default_blocking_sync_in_async(),
            blocking_io_under_lock: default_blocking_io_under_lock(),
            array_spread_in_reduce: default_array_spread_in_reduce(),
            sql_cartesian_join: default_sql_cartesian_join(),
            defer_in_loop: default_defer_in_loop(),
            goroutine_in_unbounded_loop: default_goroutine_in_unbounded_loop(),
            membership_test_in_loop: default_membership_test_in_loop(),
            hot_path_sync_io: default_hot_path_sync_io(),
        }
    }
}

impl HealthWeights {
    /// Parse a (possibly partial) TOML document into a `HealthWeights`,
    /// falling back to this type's documented default for any key left
    /// out — so a custom weights file only needs to name the penalties
    /// it actually wants to change.
    pub fn from_toml_str(s: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(s)?)
    }

    fn penalty_for(&self, kind: FindingKind) -> f64 {
        match kind {
            FindingKind::LongFunction => self.long_function,
            FindingKind::HighComplexity => self.high_complexity,
            FindingKind::TooManyParameters => self.too_many_params,
            FindingKind::GodClass => self.god_class,
            FindingKind::DuplicateCode => self.duplicate_code,
            FindingKind::NearDuplicateCode => self.near_duplicate_code,
            FindingKind::PossiblyDeadCode => self.possibly_dead_code,
            FindingKind::LowCohesion => self.low_cohesion,
            FindingKind::NestedComplexity => self.nested_complexity,
            FindingKind::BumpyRoad => self.bumpy_road,
            FindingKind::ComplexConditional => self.complex_conditional,
            FindingKind::PrimitiveObsession => self.primitive_obsession,
            FindingKind::IoInLoop => self.io_in_loop,
            FindingKind::StringConcatInLoop => self.string_concat_in_loop,
            FindingKind::ResourceConstructionInLoop => self.resource_construction_in_loop,
            FindingKind::LockInLoop => self.lock_in_loop,
            FindingKind::ListInsertZeroInLoop => self.list_insert_zero_in_loop,
            FindingKind::JsonParseInLoop => self.json_parse_in_loop,
            FindingKind::RegexCompileInLoop => self.regex_compile_in_loop,
            FindingKind::NestedLoopWithIo => self.nested_loop_with_io,
            FindingKind::NestedLoopQuadratic => self.nested_loop_quadratic,
            FindingKind::SerialAwaitInLoop => self.serial_await_in_loop,
            FindingKind::PdConcatInLoop => self.pd_concat_in_loop,
            FindingKind::BlockingSyncInAsync => self.blocking_sync_in_async,
            FindingKind::BlockingIoUnderLock => self.blocking_io_under_lock,
            FindingKind::ArraySpreadInReduce => self.array_spread_in_reduce,
            FindingKind::SqlCartesianJoin => self.sql_cartesian_join,
            FindingKind::DeferInLoop => self.defer_in_loop,
            FindingKind::GoroutineInUnboundedLoop => self.goroutine_in_unbounded_loop,
            FindingKind::MembershipTestInLoop => self.membership_test_in_loop,
            FindingKind::HotPathSyncIo => self.hot_path_sync_io,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    LongFunction,
    HighComplexity,
    TooManyParameters,
    GodClass,
    DuplicateCode,
    NearDuplicateCode,
    PossiblyDeadCode,
    LowCohesion,
    NestedComplexity,
    BumpyRoad,
    ComplexConditional,
    PrimitiveObsession,
    IoInLoop,
    StringConcatInLoop,
    ResourceConstructionInLoop,
    LockInLoop,
    ListInsertZeroInLoop,
    JsonParseInLoop,
    RegexCompileInLoop,
    NestedLoopWithIo,
    NestedLoopQuadratic,
    SerialAwaitInLoop,
    PdConcatInLoop,
    BlockingSyncInAsync,
    BlockingIoUnderLock,
    ArraySpreadInReduce,
    SqlCartesianJoin,
    DeferInLoop,
    GoroutineInUnboundedLoop,
    MembershipTestInLoop,
    HotPathSyncIo,
}

impl FindingKind {
    pub fn label(&self) -> &'static str {
        match self {
            FindingKind::LongFunction => "long-function",
            FindingKind::HighComplexity => "high-complexity",
            FindingKind::TooManyParameters => "too-many-params",
            FindingKind::GodClass => "god-class",
            FindingKind::DuplicateCode => "duplicate-code",
            FindingKind::NearDuplicateCode => "near-duplicate-code",
            FindingKind::PossiblyDeadCode => "possibly-dead-code",
            FindingKind::LowCohesion => "low-cohesion",
            FindingKind::NestedComplexity => "nested-complexity",
            FindingKind::BumpyRoad => "bumpy-road",
            FindingKind::ComplexConditional => "complex-conditional",
            FindingKind::PrimitiveObsession => "primitive-obsession",
            FindingKind::IoInLoop => "io-in-loop",
            FindingKind::StringConcatInLoop => "string-concat-in-loop",
            FindingKind::ResourceConstructionInLoop => "resource-construction-in-loop",
            FindingKind::LockInLoop => "lock-in-loop",
            FindingKind::ListInsertZeroInLoop => "list-insert-zero-in-loop",
            FindingKind::JsonParseInLoop => "json-parse-in-loop",
            FindingKind::RegexCompileInLoop => "regex-compile-in-loop",
            FindingKind::NestedLoopWithIo => "nested-loop-with-io",
            FindingKind::NestedLoopQuadratic => "nested-loop-quadratic",
            FindingKind::SerialAwaitInLoop => "serial-await-in-loop",
            FindingKind::PdConcatInLoop => "pd-concat-in-loop",
            FindingKind::BlockingSyncInAsync => "blocking-sync-in-async",
            FindingKind::BlockingIoUnderLock => "blocking-io-under-lock",
            FindingKind::ArraySpreadInReduce => "array-spread-in-reduce",
            FindingKind::SqlCartesianJoin => "sql-cartesian-join",
            FindingKind::DeferInLoop => "defer-in-loop",
            FindingKind::GoroutineInUnboundedLoop => "goroutine-in-unbounded-loop",
            FindingKind::MembershipTestInLoop => "membership-test-in-loop",
            FindingKind::HotPathSyncIo => "hot-path-sync-io",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub file: PathBuf,
    pub symbol: Option<String>,
    pub line: Option<usize>,
    pub kind: FindingKind,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct FileHealth {
    pub file: PathBuf,
    /// 0.0 (unhealthy) to 10.0 (no markers triggered).
    pub score: f64,
    pub finding_count: usize,
}

pub struct HealthReport {
    /// One entry per indexed file, sorted worst-score-first.
    pub file_scores: Vec<FileHealth>,
    pub findings: Vec<Finding>,
    pub average_score: f64,
}

impl HealthReport {
    pub fn findings_by_kind(&self) -> Vec<(FindingKind, usize)> {
        let mut counts: HashMap<&'static str, (FindingKind, usize)> = HashMap::new();
        for f in &self.findings {
            counts.entry(f.kind.label()).or_insert((f.kind, 0)).1 += 1;
        }
        let mut out: Vec<(FindingKind, usize)> = counts.into_values().collect();
        out.sort_by_key(|b| std::cmp::Reverse(b.1));
        out
    }
}

/// Score `index` using the default (fixed, hand-picked) `HealthWeights`
/// — see `analyze_with_weights` to plug in a different weight set.
pub fn analyze(index: &RepoIndex, graph: &RepoGraph) -> HealthReport {
    analyze_with_weights(index, graph, &HealthWeights::default())
}

/// Score `index` with `weights`, using no hotspot data — every marker
/// except `hot_path_sync_io`, which needs it. Equivalent to
/// `analyze_with_hotspots` with an empty hot-file set.
pub fn analyze_with_weights(
    index: &RepoIndex,
    graph: &RepoGraph,
    weights: &HealthWeights,
) -> HealthReport {
    analyze_with_hotspots(index, graph, weights, &HashSet::new())
}

/// Score `index` with `weights`, additionally reporting
/// `hot_path_sync_io` for synchronous I/O calls in files listed in
/// `hot_files` (issue #186).
///
/// This crate deliberately knows nothing about git. Deciding *which*
/// files are hot needs churn history, which lives in `repowise-git`, so
/// the caller computes that set and passes it in as plain paths —
/// keeping `repowise-health` a pure function of the index, the call
/// graph, and its inputs, exactly as it was before this marker existed.
///
/// Adding a parameter here rather than to `analyze`/`analyze_with_weights`
/// is also what keeps every existing caller (CLI/docs/dashboard/MCP/
/// server) compiling unchanged: a caller with no git data in hand keeps
/// calling the old entry point and simply never sees this marker.
pub fn analyze_with_hotspots(
    index: &RepoIndex,
    graph: &RepoGraph,
    weights: &HealthWeights,
    hot_files: &HashSet<PathBuf>,
) -> HealthReport {
    let mut findings = Vec::new();

    for file in &index.files {
        // Per repowise's own documented scope, shell scripts get a
        // narrower marker set than Full/Good-tier languages: no
        // dead-code detection (a shell function is routinely invoked
        // only from the command line, another script, or a cron job —
        // none of which this port's call graph can see, making the
        // signal too unreliable to report).
        let skip_dead_code = file.language == Language::Shell;
        for sym in &file.symbols {
            if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Method) {
                continue;
            }
            check_function_markers(
                sym,
                graph,
                skip_dead_code,
                hot_files.contains(&file.path),
                &mut findings,
            );
        }
    }

    check_god_classes(index, &mut findings);
    check_duplicate_code(index, &mut findings);
    check_near_duplicate_code(index, &mut findings);
    check_low_cohesion(index, &mut findings);

    let file_scores = score_files(index, &findings, weights);
    let average_score = if file_scores.is_empty() {
        MAX_SCORE
    } else {
        file_scores.iter().map(|f| f.score).sum::<f64>() / file_scores.len() as f64
    };

    HealthReport {
        file_scores,
        findings,
        average_score,
    }
}

#[allow(clippy::too_many_arguments)]
fn check_function_markers(
    sym: &Symbol,
    graph: &RepoGraph,
    skip_dead_code: bool,
    in_hot_file: bool,
    findings: &mut Vec<Finding>,
) {
    let length = sym.end_line.saturating_sub(sym.start_line) + 1;
    if length > LONG_FUNCTION_LINES {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::LongFunction,
            detail: format!("{length} lines (> {LONG_FUNCTION_LINES})"),
        });
    }
    if sym.complexity > HIGH_COMPLEXITY {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::HighComplexity,
            detail: format!(
                "cyclomatic complexity {} (> {HIGH_COMPLEXITY})",
                sym.complexity
            ),
        });
    }
    if sym.max_nesting_depth > HIGH_NESTING_DEPTH {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::NestedComplexity,
            detail: format!(
                "control flow nested {} levels deep (> {HIGH_NESTING_DEPTH})",
                sym.max_nesting_depth
            ),
        });
    }
    if sym.bumpy_road_bumps >= BUMPY_ROAD_MIN_BUMPS {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::BumpyRoad,
            detail: format!(
                "{} separate nested-block regions (>= {BUMPY_ROAD_MIN_BUMPS})",
                sym.bumpy_road_bumps
            ),
        });
    }
    // Threshold is already applied at extraction time (see
    // `repowise_parser::metrics::complex_conditionals`); every entry
    // here is already flagged, so no further filtering is needed.
    for cc in &sym.complex_conditionals {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(cc.line),
            kind: FindingKind::ComplexConditional,
            detail: format!(
                "condition chains {} boolean operators (>= 3)",
                cc.operator_count
            ),
        });
    }
    if sym.param_count > TOO_MANY_PARAMS {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::TooManyParameters,
            detail: format!("{} parameters (> {TOO_MANY_PARAMS})", sym.param_count),
        });
    }
    if sym.primitive_param_count >= PRIMITIVE_OBSESSION_MIN_COUNT {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::PrimitiveObsession,
            detail: format!(
                "{} bare-primitive-typed parameters (>= {PRIMITIVE_OBSESSION_MIN_COUNT})",
                sym.primitive_param_count
            ),
        });
    }
    // Already filtered to I/O-shaped calls found inside a loop body at
    // extraction time (see `repowise_parser::metrics::calls_in_loops`);
    // every entry here is already flagged, so no further filtering is
    // needed, same as `complex_conditionals` above.
    for io_call in &sym.io_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(io_call.line),
            kind: FindingKind::IoInLoop,
            detail: format!(
                "`{}` (I/O-shaped call) found inside a loop body -- consider hoisting it out",
                io_call.callee_name
            ),
        });
    }
    // Already filtered to string-append expressions found inside a loop
    // body at extraction time (see
    // `repowise_parser::metrics::string_concats_in_loops`); every entry
    // here is already flagged, same as `io_in_loop` above.
    for concat in &sym.string_concat_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(concat.line),
            kind: FindingKind::StringConcatInLoop,
            detail: format!(
                "`{}` accumulated via string concatenation inside a loop body -- \
                 consider a builder/join instead",
                concat.variable
            ),
        });
    }
    // Already filtered to expensive-resource constructor calls found
    // inside a loop body at extraction time (see
    // `repowise_parser::metrics::resource_constructions_in_loops`); every
    // entry here is already flagged, same as `io_in_loop`/
    // `string_concat_in_loop` above.
    for construction in &sym.resource_construction_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(construction.line),
            kind: FindingKind::ResourceConstructionInLoop,
            detail: format!(
                "`{}` (expensive resource construction) found inside a loop body -- \
                 consider hoisting it out",
                construction.callee_name
            ),
        });
    }
    // Already filtered to lock-acquisition calls found inside a loop body
    // at extraction time (see `repowise_parser::metrics::locks_in_loops`);
    // every entry here is already flagged, same as the other loop-body
    // markers above.
    for lock in &sym.lock_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(lock.line),
            kind: FindingKind::LockInLoop,
            detail: format!(
                "`{}` (lock acquisition) found inside a loop body -- \
                 consider acquiring it once outside the loop",
                lock.callee_name
            ),
        });
    }
    // Already filtered to index-0 list/vector inserts found inside a
    // loop body at extraction time (see
    // `repowise_parser::metrics::list_inserts_zero_in_loops`); every
    // entry here is already flagged, same as the other loop-body
    // markers above.
    for insert in &sym.list_insert_zero_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(insert.line),
            kind: FindingKind::ListInsertZeroInLoop,
            detail: format!(
                "`{}.insert(0, ...)` found inside a loop body -- O(n) per call, \
                 O(n^2) across the loop; consider appending and reversing once, or a deque",
                insert.variable
            ),
        });
    }
    // Already filtered to JSON-parsing calls found inside a loop body at
    // extraction time (see
    // `repowise_parser::metrics::json_parses_in_loops`); every entry
    // here is already flagged, same as the other loop-body markers
    // above.
    for parse in &sym.json_parse_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(parse.line),
            kind: FindingKind::JsonParseInLoop,
            detail: format!(
                "`{}` (JSON parsing) found inside a loop body -- consider parsing once \
                 outside the loop, or restructuring to parse a single batched payload",
                parse.callee_name
            ),
        });
    }
    // Already filtered to regex-compilation calls found inside a loop
    // body at extraction time (see
    // `repowise_parser::metrics::regex_compiles_in_loops`); every entry
    // here is already flagged, same as the other loop-body markers
    // above.
    for compile in &sym.regex_compile_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(compile.line),
            kind: FindingKind::RegexCompileInLoop,
            detail: format!(
                "`{}` (regex compilation) found inside a loop body -- consider hoisting it out \
                 and reusing the compiled pattern across iterations",
                compile.callee_name
            ),
        });
    }
    // Already filtered to I/O-shaped calls found at loop-nesting depth
    // 2+ at extraction time (see
    // `repowise_parser::metrics::ios_in_nested_loops`); every entry here
    // is already flagged, same as the other loop-body markers above. A
    // call reported here is also reported in `io_in_loop` above -- this
    // is intentionally a depth-2+ subset of it, not a separate pass.
    for nested_io in &sym.nested_loop_with_io {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(nested_io.line),
            kind: FindingKind::NestedLoopWithIo,
            detail: format!(
                "`{}` (I/O-shaped call) found inside a loop nested inside another loop -- \
                 potentially O(n^2) or worse; consider hoisting it out or restructuring",
                nested_io.callee_name
            ),
        });
    }
    // Already filtered to inner loops iterating an enclosing loop's own
    // collection at extraction time (see
    // `repowise_parser::metrics::quadratic_loop_nestings`); every entry
    // here is already flagged, same as the other loop markers above.
    for quadratic in &sym.nested_loop_quadratic {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(quadratic.line),
            kind: FindingKind::NestedLoopQuadratic,
            detail: format!(
                "loop iterates `{}` inside an enclosing loop over the same collection -- \
                 an O(n^2) all-pairs scan; consider a set/map lookup instead",
                quadratic.iterable
            ),
        });
    }
    // Already filtered to awaited async calls found inside a loop body
    // at extraction time (see
    // `repowise_parser::metrics::serial_awaits_in_loops`), with awaits
    // of the concurrency combinators that are the fix already excluded;
    // every entry here is already flagged, same as the other loop-body
    // markers above.
    for await_call in &sym.serial_await_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(await_call.line),
            kind: FindingKind::SerialAwaitInLoop,
            detail: format!(
                "`{}` awaited inside a loop body -- each iteration blocks on the previous one; \
                 consider batching the whole set into one concurrent await",
                await_call.callee_name
            ),
        });
    }
    // Already filtered to `pandas.concat` calls found inside a loop body
    // at extraction time (see
    // `repowise_parser::metrics::pd_concats_in_loops`); every entry here
    // is already flagged, same as the other loop-body markers above.
    for concat in &sym.pd_concat_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(concat.line),
            kind: FindingKind::PdConcatInLoop,
            detail: format!(
                "`{}` called inside a loop body -- each call copies the whole growing \
                 DataFrame, making the loop quadratic; collect the rows in a list and \
                 concatenate once after the loop",
                concat.callee_name
            ),
        });
    }
    // Already filtered to blocking calls found inside an *async*
    // function body at extraction time (see
    // `repowise_parser::metrics::blocking_calls_in_async`) -- the
    // extractor only populates this for async functions, so no
    // async-ness check is needed here.
    for blocking in &sym.blocking_sync_in_async {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(blocking.line),
            kind: FindingKind::BlockingSyncInAsync,
            detail: format!(
                "`{}` (blocking call) inside an async function -- this stalls the executor \
                 thread and every other task sharing it; use the async equivalent or move \
                 it to a blocking-friendly pool",
                blocking.callee_name
            ),
        });
    }
    // Already filtered to I/O calls made while a lock is held at
    // extraction time (see `repowise_parser::metrics`'s
    // `ios_under_lock_binding`/`ios_inside_lock_block`); every entry
    // here is already flagged.
    for io_call in &sym.blocking_io_under_lock {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(io_call.line),
            kind: FindingKind::BlockingIoUnderLock,
            detail: format!(
                "`{}` (I/O-shaped call) runs while a lock is held -- every other thread \
                 waiting on that lock blocks for the duration of the I/O; do the I/O \
                 outside the critical section",
                io_call.callee_name
            ),
        });
    }
    // Already filtered to spread-accumulator reduce callbacks at
    // extraction time (see
    // `repowise_parser::metrics::array_spreads_in_reduce`); every entry
    // here is already flagged.
    for spread in &sym.array_spread_in_reduce {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(spread.line),
            kind: FindingKind::ArraySpreadInReduce,
            detail: format!(
                "`reduce` callback returns `[...{}, ..]` -- spreading the accumulator copies \
                 the whole array every step, making a linear fold quadratic; push onto the \
                 accumulator and return it instead",
                spread.accumulator
            ),
        });
    }
    // Already filtered to SQL strings whose comma-joined tables lack
    // connecting predicates at extraction time (see
    // `repowise_parser::metrics::sql_cartesian_joins`); every entry
    // here is already flagged.
    for query in &sym.sql_cartesian_join {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(query.line),
            kind: FindingKind::SqlCartesianJoin,
            detail: format!(
                "SQL joins tables `{}` with no predicate connecting them -- this returns \
                 the full cartesian product; add an ON/WHERE join condition",
                query.tables
            ),
        });
    }
    // Already filtered to `defer`s lexically inside a loop body at
    // extraction time (see `repowise_parser::metrics::defers_in_loops`);
    // every entry here is already flagged.
    for deferred in &sym.defer_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(deferred.line),
            kind: FindingKind::DeferInLoop,
            detail: format!(
                "`defer {}(..)` inside a loop -- Go runs deferred calls when the *function* \
                 returns, not at the end of each iteration, so one is queued per iteration and \
                 the resource stays held until the whole loop finishes; move the loop body into \
                 its own function, or call `{}` explicitly instead of deferring it",
                deferred.callee_name, deferred.callee_name
            ),
        });
    }
    // Already filtered at extraction time to `go` statements in a loop
    // whose body carries no recognized bounding channel operation (see
    // `repowise_parser::metrics::unbounded_goroutines_in_loops`); every
    // entry here is already flagged.
    for launch in &sym.goroutine_in_unbounded_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(launch.line),
            kind: FindingKind::GoroutineInUnboundedLoop,
            detail: format!(
                "`go {}(..)` inside a loop with no visible concurrency bound -- this spawns one \
                 goroutine per iteration, so a large input can exhaust memory or overwhelm \
                 whatever they call; gate the launch on a buffered semaphore channel or feed a \
                 fixed-size worker pool instead",
                launch.callee_name
            ),
        });
    }
    // Already filtered at extraction time to tests whose target a local
    // binding proves is a list (see
    // `repowise_parser::metrics::list_membership_tests_in_loops`); every
    // entry here is already flagged.
    for test in &sym.membership_test_in_loop {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(test.line),
            kind: FindingKind::MembershipTestInLoop,
            detail: format!(
                "membership test against the list `{}` inside a loop -- each check scans the \
                 whole list, so the loop is O(n * m) where it looks linear; build a set/hash \
                 map once outside the loop and test against that instead",
                test.collection
            ),
        });
    }
    // The only marker in this crate built from two independent signals:
    // a structural one (a blocking call is present) and an empirical one
    // (git says this file changes often). Neither alone is a finding --
    // a blocking read in a rarely-touched setup path is fine, and a
    // hotspot with no blocking I/O has nothing to fix here.
    if in_hot_file {
        for call in &sym.sync_io_calls {
            findings.push(Finding {
                file: sym.file.clone(),
                symbol: Some(sym.name.clone()),
                line: Some(call.line),
                kind: FindingKind::HotPathSyncIo,
                detail: format!(
                    "blocking `{}` call in a git hotspot -- this file is among the most \
                     frequently changed, complexity-weighted files in the repo, so a synchronous \
                     call here is on a path that demonstrably gets exercised and edited; \
                     consider an async or buffered equivalent",
                    call.callee_name
                ),
            });
        }
    }
    if !skip_dead_code && graph.call_in_degree(&sym.id) == 0 {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::PossiblyDeadCode,
            detail: "no in-repo callers found (best-effort; may be a public API, \
                     trait impl, entry point, or a call this heuristic couldn't resolve)"
                .to_string(),
        });
    }
}

fn check_god_classes(index: &RepoIndex, findings: &mut Vec<Finding>) {
    let mut method_counts: HashMap<(PathBuf, String), usize> = HashMap::new();
    for file in &index.files {
        for sym in &file.symbols {
            if sym.kind == SymbolKind::Method {
                if let Some(parent) = &sym.parent {
                    *method_counts
                        .entry((file.path.clone(), parent.clone()))
                        .or_insert(0) += 1;
                }
            }
        }
    }
    for ((file, parent), count) in &method_counts {
        if *count <= GOD_CLASS_METHODS {
            continue;
        }
        let line = index
            .files
            .iter()
            .find(|f| &f.path == file)
            .and_then(|f| {
                f.symbols.iter().find(|s| {
                    &s.name == parent && matches!(s.kind, SymbolKind::Struct | SymbolKind::Class)
                })
            })
            .map(|s| s.start_line);
        findings.push(Finding {
            file: file.clone(),
            symbol: Some(parent.clone()),
            line,
            kind: FindingKind::GodClass,
            detail: format!("{count} methods (> {GOD_CLASS_METHODS})"),
        });
    }
}

fn check_duplicate_code(index: &RepoIndex, findings: &mut Vec<Finding>) {
    let mut groups: HashMap<u64, Vec<&Symbol>> = HashMap::new();
    for file in &index.files {
        for sym in &file.symbols {
            if let Some(hash) = sym.body_hash {
                groups.entry(hash).or_default().push(sym);
            }
        }
    }
    for group in groups.values() {
        if group.len() < 2 {
            continue;
        }
        for sym in group {
            let others: Vec<&str> = group
                .iter()
                .filter(|s| s.id != sym.id)
                .map(|s| s.name.as_str())
                .collect();
            findings.push(Finding {
                file: sym.file.clone(),
                symbol: Some(sym.name.clone()),
                line: Some(sym.start_line),
                kind: FindingKind::DuplicateCode,
                detail: format!("body identical to: {}", others.join(", ")),
            });
        }
    }
}

fn check_near_duplicate_code(index: &RepoIndex, findings: &mut Vec<Finding>) {
    for candidate in near_duplicate::find_near_duplicates(index) {
        findings.push(Finding {
            file: candidate.file,
            symbol: Some(candidate.symbol),
            line: Some(candidate.line),
            kind: FindingKind::NearDuplicateCode,
            detail: format!(
                "~{:.0}% textually similar to `{}` in {} (not identical -- \
                 see 'duplicate code' for exact matches)",
                candidate.overlap_ratio * 100.0,
                candidate.other_symbol,
                candidate.other_file.display()
            ),
        });
    }
}

fn check_low_cohesion(index: &RepoIndex, findings: &mut Vec<Finding>) {
    for candidate in lcom4::find_low_cohesion(index) {
        findings.push(Finding {
            file: candidate.file,
            symbol: Some(candidate.class),
            line: candidate.line,
            kind: FindingKind::LowCohesion,
            detail: format!(
                "{} disjoint field-access groups across {} tracked methods (>= {LOW_COHESION_MIN_COMPONENTS})",
                candidate.components, candidate.tracked_methods
            ),
        });
    }
}

fn score_files(
    index: &RepoIndex,
    findings: &[Finding],
    weights: &HealthWeights,
) -> Vec<FileHealth> {
    let mut scores: HashMap<PathBuf, f64> = index
        .files
        .iter()
        .map(|f| (f.path.clone(), MAX_SCORE))
        .collect();
    let mut counts: HashMap<PathBuf, usize> =
        index.files.iter().map(|f| (f.path.clone(), 0)).collect();

    for finding in findings {
        if let Some(s) = scores.get_mut(&finding.file) {
            *s -= weights.penalty_for(finding.kind);
        }
        if let Some(c) = counts.get_mut(&finding.file) {
            *c += 1;
        }
    }

    let mut file_scores: Vec<FileHealth> = scores
        .into_iter()
        .map(|(file, score)| FileHealth {
            score: score.clamp(0.0, MAX_SCORE),
            finding_count: counts.get(&file).copied().unwrap_or(0),
            file,
        })
        .collect();
    file_scores.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap()
            .then(b.finding_count.cmp(&a.finding_count))
            .then(a.file.cmp(&b.file))
    });
    file_scores
}
