# rusty_repo_wise

A Rust-native reimplementation inspired by [repowise](https://github.com/repowise-dev/repowise),
a codebase-intelligence platform that builds dependency graphs, git
analytics, auto-generated docs, architectural-decision tracking, and
deterministic code-health scoring for AI agents and developers.

This is a fresh, from-scratch Rust project — it does not share code or a
license with the original (AGPL-3.0) repowise.

## Scope so far

repowise is a large product with five "intelligence layers," an MCP
server, and a web dashboard. This port now builds all of that — every
piece covering a subset of the original's scope within it (see below for
specifics per layer), not full feature parity:

- Walk a codebase (respecting `.gitignore`), detect Rust, Python,
  TypeScript, JavaScript, Java, Kotlin, Go, C, C++, C#, Scala, Ruby,
  Swift, PHP, Dart, and shell (`.sh`/`.bash`/`.zsh`) files, plus
  Objective-C, R, Zig, Julia, Elm, OCaml, Crystal, Nim, and D by name
  for git-history coverage only (see "Structural-tier languages" below).
- Parse each file with tree-sitter, extracting function/method/class/struct
  definitions, imports, call expressions, and per-function metrics
  (cyclomatic complexity, parameter count, a duplicate-code body hash).
- Resolve imports and calls into a dependency graph (files and symbols as
  nodes; `Contains`/`Imports`/`Calls` edges), using directory-layout
  conventions (Rust's `mod`/crate-root rules, Python's package layout,
  TypeScript/JavaScript's relative `./`/`../` specifiers, Java/Kotlin/
  Scala's shared Maven/Gradle/sbt `src/main/java`/`src/main/kotlin`/
  `src/main/scala`-anchored package paths, Go's `go.mod`-anchored module
  paths, C/C++'s quote-form `#include "local.h"` resolved directly
  against the filesystem, C#'s best-effort folder-mirrors-namespace
  heuristic, Ruby's `require_relative` resolved directly against the
  filesystem) — **not** full compiler-grade name resolution. Ambiguous or
  external references (npm packages/JVM classpath dependencies/Go modules
  outside the local `go.mod`/C and C++ angle-form `#include <system>`
  headers/C# namespaces that don't follow the folder convention/Ruby's
  gem-based plain `require`/PHP namespaces that don't follow the folder
  convention, since there's no
  `node_modules`/classpath/Go-proxy/include-path-search/.NET-project/
  `$LOAD_PATH`/Composer-autoload resolution) are left unresolved rather
  than guessed. `.h` is deliberately not mapped to either C or C++
  (ambiguous between the two, and this port has no syntax-sniffing to
  disambiguate) — so a conventional `#include "foo.h"` split resolves
  against the filesystem at parse time but never becomes a graph edge,
  since the header itself is never indexed; only unambiguous extensions
  (`.c`, and C++'s `.cpp`/`.cc`/`.cxx`/`.hpp`/`.hh`/`.hxx`) are
  recognized. Swift's `import` is module-level rather than file-level (no
  relative-import syntax, and a module name has no file mapping without a
  full build graph) — its imports are recorded for visibility but always
  left unresolved, by design rather than as a gap. PHP has two import
  forms: `require`/`require_once`/`include`/`include_once` with a plain
  string path (resolved directly against the filesystem, same as
  C/C++/Ruby) and `use Namespace\Class;` (resolved via the same
  folder-mirrors-namespace heuristic as C#'s, not aware of Composer's
  real `composer.json` autoload mapping). Dart's relative
  `import 'local.dart'` is resolved directly against the filesystem the
  same way; `import 'package:x/y.dart'` (a pub package) has no package
  registry here to resolve against and is left unresolved, same
  tradeoff as bare npm specifiers. Shell's `source`/`.` is likewise
  resolved directly against the filesystem, including the common
  `SCRIPT_DIR="$(dirname "$0")"` / `source "$SCRIPT_DIR/helper.sh"`
  idiom for a script sourcing something relative to its own directory;
  any other variable/command-substitution in the path has no static
  value to resolve, so it's recorded but left unresolved.
- Score every file's health deterministically (0–10, no LLM/ML) from
  thirty-one rule-based markers: long functions, high cyclomatic complexity,
  oversized parameter lists, god classes, duplicate code, near-duplicate code
  (`dry_violation` — Rabin-Karp rolling-hash overlap over tokenized
  text), possibly-dead code (zero resolved callers), low cohesion
  (LCOM4 — Rust/Python/TS+JS only, see "Health scoring" below), nested
  complexity (`nested_complexity` — maximum control-flow nesting depth,
  complementing cyclomatic complexity's flat branch count), a
  "bumpy road" (`bumpy_road` — count of distinct nested-block regions,
  complementing nesting depth's single deepest-point view), complex
  conditionals (`complex_conditional` — a single `if`/`while`/etc. condition
  chaining 3+ boolean operators, Rust/Python/TS+JS only), primitive
  obsession (`primitive_obsession` — a parameter list leaning on bare
  primitives instead of domain types, Rust/TypeScript only since it needs
  declared parameter types), and all nineteen of repowise's Performance-signal
  cluster: I/O in a loop (`io_in_loop` — a known file/network/database
  call found inside a loop body where hoisting it above the loop is
  usually possible, Rust/Python/TS+JS only), string concatenation
  in a loop (`string_concat_in_loop` — `+=`/`s = s + other`/
  `.push_str(..)` accumulating onto a string variable inside a loop
  body, quadratic string-building cost since each append reallocates
  and copies the whole string so far, Rust/Python/TS+JS only), expensive
  resource construction in a loop (`resource_construction_in_loop` — a
  known expensive-to-construct resource, e.g. an HTTP client or
  connection/thread pool, built inside a loop body where hoisting it
  above the loop is usually possible, Rust/Python/TS+JS only), lock
  acquisition in a loop (`lock_in_loop` — a mutex/lock acquisition
  (`.lock()`/`.acquire()`) happening inside a loop body instead of once
  outside it, Rust/Python/TS+JS only), and an index-0 list/vector insert
  in a loop (`list_insert_zero_in_loop` — `.insert(0, ...)` inside a loop
  body, O(n) per call and O(n²) across the loop since it shifts every
  element, versus appending and reversing once or using a deque; Rust and
  Python only, unlike the other six Performance-signal markers), and
  JSON parsing in a loop (`json_parse_in_loop` — a known JSON-deserializing
  call (`serde_json::from_str`/`json.loads`/`JSON.parse`) found inside a
  loop body where parsing once outside the loop, or restructuring to parse
  a single batched payload, is usually possible, Rust/Python/TS+JS only),
  and regex compilation in a loop (`regex_compile_in_loop` — a known
  regex-construction call (`Regex::new`/`re.compile`/`new RegExp`) found
  inside a loop body where compiling the pattern once outside the loop is
  usually possible, Rust/Python/TS+JS only), and I/O in a nested loop
  (`nested_loop_with_io` — an `io_in_loop` call found at loop-nesting
  depth 2 or deeper, potentially O(n²) or worse rather than O(n), so it's
  a depth-2+ subset of `io_in_loop` reported under both markers rather
  than a separate detection pass, Rust/Python/TS+JS only), and a
  quadratic nested loop (`nested_loop_quadratic` — an inner loop
  iterating the same collection as an enclosing loop, the accidental
  all-pairs O(n²) scan that's usually replaceable with a set/map lookup,
  Rust/Python/TS+JS only), and a serial await in a loop
  (`serial_await_in_loop` — an awaited async call inside a loop body, so
  each iteration blocks on the previous one instead of the whole batch
  running concurrently via `Promise.all`/`join_all`/`asyncio.gather`;
  awaits *of* those combinators are themselves excluded, Rust/Python/TS+JS
  only), and a pandas concat in a loop (`pd_concat_in_loop` —
  `pd.concat(..)` accumulating rows one at a time inside a loop body,
  which copies the whole growing DataFrame every iteration and makes the
  loop quadratic; Python only, since pandas has no equivalent in this
  port's other languages), and a blocking sync call in an async function
  (`blocking_sync_in_async` — a known blocking call (`std::thread::sleep`,
  `time.sleep`, `requests.get`, blocking `std::fs`/`open`) inside an
  `async fn`/`async def`, which stalls the executor thread and every
  other task sharing it; Rust and Python only, and the one marker in
  this cluster keyed on the enclosing *function* being async rather than
  on an enclosing loop), and blocking I/O under a lock
  (`blocking_io_under_lock` — an `io_in_loop`-table call made while a
  mutex/lock is held, serializing every other thread waiting on that lock
  behind however long the I/O takes; Rust and Python only), and an
  array spread in a reduce (`array_spread_in_reduce` — a `.reduce(..)`
  callback returning `[...acc, x]` instead of mutating and returning the
  accumulator, which copies the whole array every step and turns a
  linear fold quadratic; TypeScript/JavaScript only), and a SQL
  cartesian join (`sql_cartesian_join` — a query string listing several
  comma-joined tables with no predicate connecting them, silently
  returning `n × m` rows; a coarse text scan of string literals, not a
  SQL parse, in Rust/Python/TS+JS), and a `defer` in a loop
  (`defer_in_loop` — a Go `defer` statement inside a loop body, where the
  deferred call runs at the enclosing *function's* return rather than at
  the end of the iteration that queued it, so every resource stays held
  until the whole loop finishes; Go only), and an unbounded goroutine
  launch in a loop (`goroutine_in_unbounded_loop` — a Go `go` statement
  inside a loop body whose only bound would be the iteration count, so a
  large input spawns an unbounded goroutine fan-out; suppressed when the
  loop body carries a channel send/receive outside the launch, the
  acquire half of the standard semaphore/worker-pool idiom; Go only),
  and a membership test against a list in a loop
  (`membership_test_in_loop` — `x in xs`/`xs.contains(&x)`/
  `xs.includes(x)` inside a loop body where a local binding shows `xs`
  is a list rather than a set/map, so each check is an O(n) scan and the
  loop is O(n × m) where it reads as linear; Rust/Python/TS+JS only),
  and synchronous I/O on a hot path (`hot_path_sync_io` — a blocking
  file/network call in a function whose file the git-hotspot analytics
  rank among the repo's most churn-and-complexity-heavy, combining a
  structural signal with an empirical one; Rust/Python/TS+JS only)
  — except for shell scripts,
  which are deliberately exempt from the dead-code
  marker: a shell function is routinely invoked only from the command
  line, another script, or a cron job, none of which this port's call
  graph can see, making the signal too unreliable to report for that
  language.
- Derive git-history analytics — churn, hotspot score (churn × complexity),
  bug-fix commit frequency, co-change coupling, and per-author line
  ownership — by shelling out to `git log`/`git blame`, joined against the
  index for complexity data.
- Generate a deterministic, template-based markdown "wiki" page per file
  under `.repowise/wiki/` — symbol list, resolved dependencies/dependents,
  and health findings — with per-file freshness tracking (no LLM prose).
- Mine architectural decisions from `docs/adr/*.md` files and decision-like
  commit messages, link each to the files/symbols it mentions, and track
  supersession via an ADR's `Status: Superseded by ADR-XXXX` line.
- Expose `get_overview`/`search_codebase`/`get_context`/`get_risk`/
  `get_change_risk`/`get_symbol`/`get_why`/`get_dead_code` as MCP tools
  over stdio (the official `rmcp` SDK), so an agent can pull complete
  context (including git-history risk data, a deterministic per-commit
  risk score, a symbol's raw source, the architectural decisions behind a
  file, and confidence-tiered dead-code candidates) for a file, a change,
  a single symbol, "why was this built this way", or "what looks unused"
  in one round-trip instead of piecing it together itself.
- Generate a static-site dashboard (one self-contained HTML page, no
  server, no JS build step) covering overview stats, code health,
  hotspots, and mined decisions — regenerate by re-running the command.
- Persist the index to `.repowise/index.json` and query it from the CLI.

Only Rust, Python, TypeScript, JavaScript, Java, Kotlin, Go, C, C++, C#,
Scala, Ruby, Swift, PHP, Dart, and shell scripts are tree-sitter parsed
(symbols/imports/calls/complexity). Nine more — Objective-C, R, Zig,
Julia, Elm, OCaml, Crystal, Nim, and D (issue #70's "Structural tier")
— are recognized by name and get git-history coverage (`hotspots`/
`ownership`/`coupled`, churn/blame/co-change) but no symbol extraction
at all: no grammar exists for them, so their hotspot score is always
`0` (churn × 0 complexity) and they carry no imports/calls to resolve.
Every other repowise language is unimplemented. The health scorer now
implements 31 distinct markers. That exceeds repowise's headline "~25
markers" because that figure counts the Performance-signal work as a
single item, while this port implements its pattern checks individually
(issue #72 alone enumerates 19). The remaining gap is not a count but a
kind: the ML-calibrated organizational-signal markers are still deferred
— see "Health scoring" below for what is implemented and why the rest
wait. `repowise-docs`'s
per-file wiki pages stay deterministic-only, but an opt-in `repowise-llm`
crate can layer an LLM-written summary on top of each one (`repowise
generate`, see "LLM-assisted wiki summaries" below) — a first, narrow slice
of what was previously a fully-deferred LLM tier; RAG chat and refactor-plan
codegen remain deferred. ADR mining is also not fully ported (only 6 of the
original's 8 decision sources are implemented —
see "Architectural decision mining" below). The MCP server covers 8 of
the original's ~10 tools — see "MCP server" below for which and why. The
dashboard is one static page with no per-file drill-down or live search
— see "Dashboard" below for what a richer version would need.

## Crates

- `repowise-core` — shared data model (`Symbol`, `FileRecord`, `RepoIndex`,
  etc.), `.gitignore`-aware file discovery, and JSON index persistence.
- `repowise-parser` — tree-sitter-based extraction for Rust, Python,
  TypeScript, JavaScript, Java, Kotlin, Go, C, C++, C#, Scala, Ruby,
  Swift, PHP, Dart, and shell scripts, including per-function
  complexity/nesting-depth/bumpy-road/param-count/body-hash metrics, plus
  per-method `self`/`this` field-access tracking for Rust/Python/TS+JS
  (feeds LCOM4), per-condition boolean-operator-chain detection for
  Rust/Python/TS+JS (feeds `complex_conditional`), declared-parameter-type
  extraction for Rust/TypeScript (feeds `primitive_obsession`), and a
  shared loop-classifier for Rust/Python/TS+JS feeding a per-language
  I/O-callee-name table (`io_in_loop`), a per-language string-append-
  expression classifier (`string_concat_in_loop`), a per-language
  expensive-resource-constructor-name table (`resource_construction_in_loop`),
  a per-language lock-acquisition-callee-name table (`lock_in_loop`),
  (Rust/Python only) an index-0-list-insert-call classifier
  (`list_insert_zero_in_loop`), a per-language JSON-parse-callee-name
  table (`json_parse_in_loop`), a per-language regex-compile-callee-name
  table (`regex_compile_in_loop`), a shared loop-nesting-*depth*
  classifier reusing the I/O-callee-name table above
  (`nested_loop_with_io`), a per-language loop-iterable normalizer
  feeding a shared same-collection comparison
  (`nested_loop_quadratic`), and a per-language awaited-call extractor
  with a concurrency-combinator exclusion table
  (`serial_await_in_loop`), and (Python only) a qualified
  pandas-concat-callee table (`pd_concat_in_loop`), plus (Rust/Python) an
  `is_async_fn` classifier feeding a whole-body blocking-call scan
  (`blocking_sync_in_async`), and (Rust/Python) two lock-scope
  extractors reusing the I/O-callee-name table
  (`blocking_io_under_lock`), plus (TS/JS) a `reduce`-callback
  return-shape extractor (`array_spread_in_reduce`), and a
  language-agnostic string-literal SQL scanner
  (`sql_cartesian_join`), and (Go) the first Go loop classifier plus a
  `defer`-statement callee extractor (`defer_in_loop`), and (Go) a
  channel-operation bound classifier scoped per enclosing loop
  (`goroutine_in_unbounded_loop`), and (Rust/Python/TS+JS) a
  binding-shape collection-kind classifier paired with a membership-test
  extractor (`membership_test_in_loop`), and (Rust/Python/TS+JS) a
  whole-body rescan of the same I/O-callee table
  (`hot_path_sync_io`).
- `repowise-graph` — builds the dependency graph from a `RepoIndex` and
  answers overview/search/deps/call-in-degree queries.
- `repowise-health` — deterministic code-health scoring built on top of
  the parsed metrics and the call graph, including LCOM4 low-cohesion
  detection over per-class field-access data and Rabin-Karp near-duplicate
  detection over source text re-read from disk.
- `repowise-git` — git-history analytics (churn, hotspots, bug-fix
  frequency, co-change coupling, ownership), computed fresh from `git
  log`/`git blame` each time it's queried rather than cached in the index.
- `repowise-docs` — deterministic per-file markdown documentation pages
  rendered from the index/graph/health data, with content-hash-based
  freshness tracking.
- `repowise-adr` — architectural-decision mining from ADR files,
  decision-like commit messages, decision-like merged PR bodies (via the
  GitHub API, opt-in behind a token env var), decision-like code
  comments, inline decision markers, and keep-a-changelog-style
  CHANGELOG sections, linked to the files/symbols they mention.
- `repowise-mcp` — an MCP server (via the official `rmcp` SDK) exposing
  the index/graph/health/git-analytics/mined-decisions data, plus a
  deterministic per-commit change-risk score and confidence-tiered
  dead-code candidates, as agent-facing tools over stdio.
- `repowise-dashboard` — a static-site dashboard rendered from the
  overview/health/hotspot/decision data the other layers compute.
- `repowise-llm` — the one crate that talks to an LLM: an opt-in,
  OpenAI-compatible chat-completions client (works against a self-hosted
  [`rusty_provider`](https://github.com/baileyrd/rusty_provider) instance
  or any other OpenAI-compatible endpoint) that layers a written summary
  on top of each `repowise-docs` wiki page.
- `repowise-server` — a live axum HTTP server (JSON API + static-asset
  serving) for the dashboard's Phase 0 live-server pivot — see "Live
  dashboard server" below. `repowise-web` (its Leptos/WASM frontend
  companion) lives alongside it in `crates/` but is deliberately **not**
  a member of this repo's Cargo workspace, since it only builds for
  `wasm32-unknown-unknown`.
- `repowise-workspace` — multi-repo workspace configuration (issue #64's
  first slice): parses a small standalone TOML file naming a set of
  repo roots, and reports each one's indexed status. See "Multi-repo
  workspace support" below.
- `repowise-cli` — the `repowise` binary tying it together.

## Usage

```sh
cargo build --release

repowise init [PATH]              # build a fresh index (default PATH: .)
repowise update [PATH]             # re-index (currently a full re-index)
repowise overview [PATH]           # summary stats: languages, symbols, edges
repowise search "<query>"  [PATH]  # substring search over symbol names
repowise deps <FILE> [PATH]        # a file's resolved dependencies/dependents
repowise health [PATH]             # code-health KPIs and lowest-scoring files
                                    #   --weights <FILE> to override penalty weights (partial TOML)
repowise export --out <DIR> [PATH] # export the wiki tree, or the dependency graph as JSON Graph Format
                                    #   --format <markdown|json-graph> (default markdown), --force for a non-empty target
repowise coverage add <REPORTS...> # ingest LCOV report(s), merging into existing coverage
                                    #   --replace to discard prior coverage, --path <DIR>
repowise coverage status [PATH]    # per-file coverage summary + per-test map stats
repowise impacted-tests [REVSPEC] [PATH]  # tests a change provably exercises (needs coverage + per-test map)
repowise doctor [PATH]             # setup diagnostics: git, history depth, index, optional env vars
repowise hook install|uninstall|status [PATH]  # post-commit hook that refreshes the index after each commit
repowise status [PATH]             # index freshness: stale/missing files, wiki + dashboard state
                                    #   --verbose to list the individual stale/missing files
repowise dead-code [PATH]          # confidence-tiered dead-code candidates
                                    #   --min-confidence <low|medium|high> (default low), --limit <N> (default 50)
repowise risk [REVSPEC] [PATH]     # diff-shape risk score for a commit or `base..head` range (default HEAD)
repowise hotspots [PATH]           # files ranked by churn × complexity
repowise ownership <FILE> [PATH]   # per-author line ownership (git blame)
repowise coupled <FILE> [PATH]     # files that most often change alongside it
repowise docs [PATH]               # generate per-file wiki pages under .repowise/wiki
repowise decisions [PATH]          # mined ADRs + decision-like commits, with linked files
                                    #   --for-file <FILE> to filter to one file
repowise serve [PATH]               # run an MCP server over stdio (get_overview/search_codebase/get_context/get_risk/get_change_risk/get_symbol/get_why/get_dead_code/list_repos/get_architecture/get_blast_radius)
                                     #   --workspace <FILE> to opt into list_repos/get_architecture/get_blast_radius (see "Multi-repo workspace support")
repowise dashboard [PATH]           # generate a static HTML dashboard under .repowise/dashboard
repowise generate [PATH]            # add an LLM-written summary to each wiki page (opt-in, requires prior `docs`)
repowise serve-dashboard [PATH]      # run a live dashboard server (JSON API + optional static frontend)
                                     #   --addr <ADDR> (default 127.0.0.1:8080), --static-dir <DIR> (repowise-web's `trunk build` output)
                                     #   --workspace <FILE> to opt into the Workspace/Workspace Co-Changes/System Map/Conformance/Contracts sections
repowise workspace-repos --workspace <FILE>  # list every repo in a workspace TOML file + indexed status
repowise workspace-co-changes --workspace <FILE>  # each workspace repo's own most-coupled file pairs
                                     #   --top <N> (default 10) how many pairs to list per repo
repowise workspace-architecture --workspace <FILE>  # cross-repo Rust `use` resolution: which repos depend on which
repowise workspace-blast-radius --workspace <FILE> --repo <NAME> --file <FILE>  # direct cross-repo importers of one file
repowise workspace-conformance --workspace <FILE>  # circular cross-repo dependencies (repo A imports B imports A)
repowise workspace-contracts --workspace <FILE>  # regex-based HTTP producer/consumer route matching across repos
```

## Health scoring

`repowise health` requires a prior `init`/`update`. Each file starts at a
score of 10.0 and loses points for every marker triggered in it, clamped
to `[0, 10]`:

| Marker | Threshold | Penalty |
|---|---|---|
| Long function | > 50 lines | −0.5 |
| High cyclomatic complexity | > 10 | −1.0 |
| Too many parameters | > 6 | −0.3 |
| God class | > 15 methods | −1.5 |
| Duplicate code | body hash matches another symbol's | −0.5 |
| Near-duplicate code (`dry_violation`) | >= 50% tokenized-window overlap with another symbol | −0.3 |
| Possibly dead code | 0 resolved callers | −0.2 |
| Low cohesion (LCOM4) | >= 2 disjoint field-access groups | −1.0 |
| Nested complexity (`nested_complexity`) | control flow nested > 4 levels deep | −1.0 |
| Bumpy road (`bumpy_road`) | >= 3 separate nested-block regions | −0.5 |
| Complex conditional (`complex_conditional`) | single condition chains >= 3 boolean operators | −0.3 |
| Primitive obsession (`primitive_obsession`) | >= 3 bare-primitive-typed parameters | −0.3 |
| I/O in loop (`io_in_loop`) | a known I/O-shaped call found inside a loop body | −0.3 |
| String concat in loop (`string_concat_in_loop`) | a string-append expression accumulating inside a loop body | −0.3 |
| Resource construction in loop (`resource_construction_in_loop`) | a known expensive-to-construct resource built inside a loop body | −0.3 |
| Lock in loop (`lock_in_loop`) | a mutex/lock acquisition happening inside a loop body | −0.3 |
| List insert-zero in loop (`list_insert_zero_in_loop`) | `.insert(0, ...)` on a list/vector found inside a loop body | −0.3 |
| JSON parse in loop (`json_parse_in_loop`) | a known JSON-deserializing call found inside a loop body | −0.3 |
| Regex compile in loop (`regex_compile_in_loop`) | a known regex-compilation call found inside a loop body | −0.3 |
| Nested loop with I/O (`nested_loop_with_io`) | an `io_in_loop` call found at loop-nesting depth 2+ | −0.6 |
| Nested loop quadratic (`nested_loop_quadratic`) | an inner loop iterating the same collection as an enclosing loop | −0.6 |
| Serial await in loop (`serial_await_in_loop`) | an awaited async call inside a loop body (excluding awaits of concurrency combinators) | −0.3 |
| pandas concat in loop (`pd_concat_in_loop`) | a `pd.concat(..)` call inside a loop body | −0.6 |
| Blocking sync in async (`blocking_sync_in_async`) | a known blocking call inside an `async fn`/`async def` body | −0.6 |
| Blocking I/O under lock (`blocking_io_under_lock`) | a known I/O-shaped call made while a mutex/lock is held | −0.6 |
| Array spread in reduce (`array_spread_in_reduce`) | a `.reduce(..)` callback returning an array literal that spreads its own accumulator | −0.6 |
| SQL cartesian join (`sql_cartesian_join`) | a SQL string with comma-joined tables and too few connecting predicates | −0.6 |
| Defer in loop (`defer_in_loop`) | a Go `defer` statement inside a loop body | −0.6 |
| Goroutine in unbounded loop (`goroutine_in_unbounded_loop`) | a Go `go` statement inside a loop body with no channel-based concurrency bound | −0.6 |
| Membership test in loop (`membership_test_in_loop`) | an `x in xs`-shaped test inside a loop where a local binding shows `xs` is a list, not a set | −0.6 |
| Hot-path sync I/O (`hot_path_sync_io`) | a blocking I/O call in a function whose file ranks among the repo's top git hotspots | −0.3 |

"Possibly dead code" is never applied to shell scripts (`Language::Shell`)
— a shell function is routinely invoked only from the command line,
another script, or a cron job, none of which this port's call graph can
see, so the signal is too unreliable to report for that language. All
other markers still apply to shell the same as everywhere else.

**Penalty weights are pluggable** (`repowise_health::HealthWeights`),
not hardcoded — the table above is this type's `Default`, and every
caller (`repowise health`/`repowise docs`/`repowise dashboard`/the MCP
server) still gets exactly those values unless it opts into something
else. `repowise health --weights <FILE>` loads a (possibly partial) TOML
file of overrides — an omitted key keeps its documented default — e.g.:

```toml
# only overriding two of the thirty-one; everything else keeps its default
high_complexity = 2.0
god_class = 3.0
```

This is a precursor for the ML-calibrated weights repowise itself uses
(see issue #62), not the calibration itself — a real calibrated weight
set still needs a labeled defect corpus and a training pipeline this
port doesn't have, and sourcing that data is still an open question.
What this abstraction unblocks is having *somewhere* to plug calibrated
numbers into once they exist, without touching any scoring logic.

All of these come from data already computed by `repowise-parser`
(per-symbol line span, complexity, param count, body hash) and
`repowise-graph` (call-graph in-degree) — no new heuristics are hidden
inside the scorer itself. "Possibly dead code" and "duplicate code" are
intentionally low-weighted since they inherit the graph layer's
best-effort call/import resolution: a symbol can look uncalled just
because a call site couldn't be resolved (trait dispatch, dynamic
dispatch, an external caller), not because it's truly unused.

**Low cohesion (LCOM4)** builds a per-class graph — methods are nodes,
an edge connects two methods that both access at least one common field
— and flags a class whose field-touching methods split into 2+ disjoint
groups (no shared field access between groups at all). Field-access
extraction (`self`/`this` field reads/writes) is currently implemented
for **Rust, Python, and TypeScript/JavaScript only** — the three
languages issue #51's own acceptance criteria named explicitly, out of
the 16 languages this port otherwise parses; the other 13 have an empty
field-access list per file and are silently skipped for this one marker
(not enough data, not "cohesive"), not flagged. A method that never
touches a field of its own (a pure delegator, a static-style helper) is
excluded from the per-class graph entirely rather than counted as its
own singleton component — otherwise nearly any real-world class would
trip this marker. Extending field-access tracking to the remaining
languages is a natural follow-up, not done here.

**Nested complexity (`nested_complexity`)** measures maximum control-flow
nesting depth (if/for/while/etc. nested inside each other) per function,
complementing cyclomatic complexity: a function with 10 sequential ifs
and one with the same 10 ifs nested inside each other score identically
on cyclomatic complexity but read very differently, and only nesting
depth tells them apart. Computed by `repowise-parser::metrics::max_nesting_depth`
alongside the existing `cyclomatic_complexity` — same recursive
decision-node classification per language, just tracking depth reached
rather than a flat count — for **all 16 parsed languages** (unlike
LCOM4, this needed no new per-language extraction logic, since every
language's `is_decision` classification already existed for cyclomatic
complexity).

**Bumpy road (`bumpy_road`)** complements `nested_complexity`: rather
than the single deepest point reached, it counts how many *separate*
nested-block regions occur within one function — three separate
two-level-deep blocks read worse than one two-level-deep block, even
at the same max nesting depth, and `max_nesting_depth` alone can't
tell them apart. Computed by `repowise-parser::metrics::bumpy_road_bumps`,
also alongside `cyclomatic_complexity`/`max_nesting_depth` for all 16
languages. Counting rule: only *leaf* decision nodes count (a decision
node with no further decision node nested inside it, before hitting a
nested-function boundary) that reach a nesting depth of at least 2 —
a linear chain (`if` containing `if` containing `if`) has exactly one
leaf and counts as a single bump, not three, since it's one deep block
rather than several scattered ones; three separate sibling `if`s each
with one level of nesting inside have three leaves and count as three
bumps. Flagged at 3+ bumps.

**Complex conditional (`complex_conditional`)** flags a single `if`/
`while`/etc. condition that chains 3+ boolean operators (`&&`/`||` in
Rust/JS/TS, `and`/`or` in Python) — unlike `nested_complexity` and
`bumpy_road`, which are Symbol-level aggregate scalars, this marker needs
to point at the *specific condition*, not just the enclosing function, so
each flagged condition is its own `Finding` with its own line number
(`Symbol::complex_conditionals: Vec<ComplexConditionalRef>`, each entry
carrying the condition's own `line` and its `operator_count`). Extraction
is implemented for **Rust, Python, and TypeScript/JavaScript only** — the
same three languages LCOM4 and near-duplicate detection require new
per-language grammar logic for — via a `condition_of` closure per language
that pulls the `condition` sub-expression out of an `if`/`while`/etc. node,
and a separate `is_boolean_operator` closure (deliberately distinct from
each language's existing `is_decision` classifier) that counts chained
boolean operators within just that condition's own subtree, not the whole
function body. The other 13 languages have no per-language
`condition_of`/`is_boolean_operator` logic yet and so never produce any
entries for this marker.

**Primitive obsession (`primitive_obsession`)** flags a function/method
whose declared parameters lean on bare primitives (`i32`/`bool`/`String`
and language equivalents) rather than small domain-specific types — the
classic "primitive obsession" smell, where a handful of loosely-related
primitive values would read better bundled into their own type. This
needs actual declared parameter *types*, which only exist for
statically-typed languages in this port's model, so it's implemented for
**Rust and TypeScript only** for this first pass (`Symbol` gains
`primitive_param_count: usize`, counting declared parameters whose type
resolves to a bare primitive). For Rust, a leading `&`/`&mut`/lifetime
reference prefix is stripped before classifying (`&str` and `String`
both count), and `String`/`str` are included alongside the scalar keyword
types even though `String` isn't a `Copy` primitive in Rust's own type
system — the smell targets overused strings/ints/bools, not Rust's
`Copy` boundary. For TypeScript, only `string`/`number`/`boolean` count
(not `any`/`unknown`/`void`/etc.). The other 14 parsed languages
(including Python/JavaScript, which lack static type annotations in the
common case and would need type inference this port doesn't have) get an
empty parameter-type extraction and so never trigger this marker;
extending it to the remaining statically-typed languages (Java, Kotlin,
Go, C, C++, C#, Scala, Swift, Dart) is a natural follow-up, not done here.

**I/O in loop (`io_in_loop`)** flags a known file/network/database call
found anywhere inside a loop body — the first slice of repowise's
Performance-signal cluster (issue #72), and, like LCOM4/`complex_conditional`,
implemented for **Rust, Python, and TypeScript/JavaScript only** for this
first pass. Needs two new pieces of per-language logic: an `is_loop`
classifier (a subset of each language's existing `is_decision` node
kinds — loops only, not `if`/`match`/etc., which branch but don't
repeat) and a small fixed table of I/O-shaped callee names (e.g.
`read_to_string`/`execute`/`query` for Rust, `read`/`execute`/`urlopen`
for Python, `readFileSync`/`fetch`/`query` for TypeScript/JavaScript).
`repowise-parser::metrics::calls_in_loops` walks a function body tracking
a single "currently inside a loop" flag (so a call nested inside two
loops is still only reported once, at its own line, rather than once per
enclosing loop), matching each call node's callee name against that
table (`Symbol::io_in_loop: Vec<IoInLoopRef>`, each entry carrying the
call's own `line` and `callee_name`). Like `unresolved_import_stems` and
`repowise-workspace::contracts`'s route-matching table, this is
deliberately coarse and heuristic: matching on a call's last name/segment
means it can't tell a database cursor's `.execute(..)` from an unrelated
`execute` method on some other type, and it can't recognize I/O hidden
behind a wrapper function the table doesn't name. The other 13 languages
have no `is_loop`/I/O-callee-table logic yet and so never produce any
entries for this marker.

**String concat in loop (`string_concat_in_loop`)** flags a string-append
expression accumulating onto a variable found anywhere inside a loop body
— the second slice of the Performance-signal cluster, reusing `io_in_loop`'s
`is_loop` classifier and, like it, implemented for **Rust, Python, and
TypeScript/JavaScript only**. Each language recognizes two shapes: a
compound `+=` assignment onto a bare identifier, and a reassignment of the
shape `s = s + other` (an `assignment` whose right side is a `+`
binary expression naming the left-hand identifier on either side). Rust
additionally recognizes `s.push_str(other)` — a `call_expression` whose
callee is a `push_str` method on a bare identifier — since Python/JS have
no mutating string-append method (their strings are immutable, so `+=`/
reassignment are the only shapes). `repowise-parser::metrics::string_concats_in_loops`
mirrors `calls_in_loops`'s "currently inside a loop" tracking exactly, just
matching a different per-language classifier
(`Symbol::string_concat_in_loop: Vec<StringConcatInLoopRef>`, each entry
carrying the append's own `line` and the appended-onto `variable` name).
Repeated string concatenation is a well-known quadratic-time trap: each
append reallocates and copies the whole string built so far, so this is a
genuine performance-risk signal, not just a style nit. The other 13
languages have no per-language classifier yet and so never produce any
entries for this marker.

**Resource construction in loop (`resource_construction_in_loop`)** flags
construction of a known expensive resource (an HTTP client, a connection/
thread pool) found anywhere inside a loop body — the third slice of the
Performance-signal cluster, reusing `io_in_loop`'s `is_loop` classifier
and, like it, implemented for **Rust, Python, and TypeScript/JavaScript
only**. The tricky part is distinguishing "expensive resource
construction" from ordinary object construction: a small fixed table of
constructor names per language (e.g. `HttpClient::new`/`Client::new`/
`ThreadPool::new` for Rust, `Session`/`ThreadPoolExecutor`/`Pool` for
Python, `HttpClient`/`Client`/`ThreadPool`/`Pool` for TypeScript/
JavaScript) is matched against each call/constructor node's callee name,
deliberately excluding cheap allocation constructors (`Vec::with_capacity`,
`String::new`) and regex construction (`Regex::new`/`re.compile`/
`new RegExp`, reserved for `regex_compile_in_loop`, issue #188, so the
two markers don't double-flag the same call once both exist). Rust's
table match is on the *qualified* `Type::method` path rather than
`call_target_name`'s plain last-segment match, since a bare method name
like `new` alone is far too generic (`Vec::new()` must not match);
Python/JS match on the bare constructor name since class names are
already distinctive enough there. `repowise-parser::metrics::resource_constructions_in_loops`
reuses the exact same "currently inside a loop" tracking shape as
`io_in_loop`/`string_concat_in_loop` (`Symbol::resource_construction_in_loop:
Vec<ResourceConstructionInLoopRef>`, each entry carrying the call's own
`line` and `callee_name`). Like the other two, this is deliberately
coarse and heuristic: it can't recognize an expensive constructor hidden
behind a type alias or a wrapper function the table doesn't name. The
other 13 languages have no per-language table yet and so never produce
any entries for this marker.

**Lock in loop (`lock_in_loop`)** flags a mutex/lock acquisition call
found anywhere inside a loop body — the fourth slice of the
Performance-signal cluster, reusing `io_in_loop`'s `is_loop` classifier
and, like it, implemented for **Rust, Python, and TypeScript/JavaScript
only**. Repeated per-iteration lock/unlock churn is usually avoidable by
acquiring the lock once outside the loop instead. A small fixed table of
lock-acquisition method names per language is matched against each call
node's callee name: Rust's `Mutex::lock`/`try_lock` (deliberately
excluding `RwLock::read`/`write`, since those bare method names are far
too generic on their own — the `Read`/`Write` trait methods, plain
field getters/setters, etc. share the same names, and this port has no
type information to know a given receiver is actually an `RwLock`);
Python's `threading.Lock`/`RLock`'s `.acquire()` (the `with lock:` shape
isn't recognized — distinguishing a lock context manager from any other
`with` statement would need type information this port doesn't have);
and the same `.acquire()` shape for TypeScript/JavaScript, mirroring
common userland lock libraries (e.g. `async-mutex`) since JS has no
native mutex. `repowise-parser::metrics::locks_in_loops` reuses the same
"currently inside a loop" tracking shape as the other three loop-body
markers (`Symbol::lock_in_loop: Vec<LockInLoopRef>`, each entry carrying
the call's own `line` and `callee_name`). The other 13 languages have no
per-language table yet and so never produce any entries for this marker.

**List insert-zero in loop (`list_insert_zero_in_loop`)** flags
`.insert(0, ...)` on a list/vector found anywhere inside a loop body —
the fifth slice of the Performance-signal cluster, reusing `io_in_loop`'s
`is_loop` classifier, but unlike the other four Performance-signal
markers, implemented for **Rust and Python only** (this marker's own
scope doesn't extend to TypeScript/JavaScript). Inserting at index 0
shifts every existing element, so it's O(n) per call and O(n²) across
the whole loop — building a list "in reverse order" via repeated
front-insertion is a common accidental-quadratic pattern, versus
appending and reversing once, or using a deque. Unlike the callee-name-table
markers above, this classifier needs to inspect the call's *arguments*
too (the first argument must be the literal `0`), not just the callee
name, so it's a single combined per-language classifier rather than a
name-table `filter` step: Rust's `.insert(0, ...)` on an identifier
receiver (covering both `Vec::insert`/`VecDeque::insert`, since this
port has no type information to distinguish the two), and Python's
`list.insert(0, ...)` the same way.
`repowise-parser::metrics::list_inserts_zero_in_loops` reuses the same
"currently inside a loop" tracking shape as the other loop-body markers
(`Symbol::list_insert_zero_in_loop: Vec<ListInsertZeroInLoopRef>`, each
entry carrying the call's own `line` and the receiver's `variable`
name). The other 14 languages have no per-language classifier yet and so
never produce any entries for this marker.

**JSON parse in loop (`json_parse_in_loop`)** flags a known
JSON-deserializing call found anywhere inside a loop body — the sixth
slice of the Performance-signal cluster, reusing `io_in_loop`'s `is_loop`
classifier and, like it, implemented for **Rust, Python, and
TypeScript/JavaScript only**. Parsing the same (or a similarly-shaped)
JSON payload once per loop iteration is usually avoidable by parsing
once outside the loop, or by restructuring the call producing the loop's
input to hand back a single already-parsed/batched payload instead. A
small fixed table of JSON-parsing call paths per language is matched
against each call node's callee name: Rust's `serde_json::from_str`/
`from_slice`, Python's `json.loads`/`json.load`, and TypeScript/
JavaScript's `JSON.parse`. All three tables match on a *qualified*
two-segment path (`module::function`/`object.method`/`Object.method`)
rather than the bare last-segment name `call_target_name` uses for the
general call graph, since a bare name alone is far too generic here —
`from_str`/`loads`/`load`/`parse` collide with unrelated methods on
other types (`FromStr` impls, `pickle.load`/`yaml.load`, `Date.parse`,
etc.) that have nothing to do with JSON. `repowise-parser::metrics::json_parses_in_loops`
reuses the same "currently inside a loop" tracking shape as the other
loop-body markers (`Symbol::json_parse_in_loop: Vec<JsonParseInLoopRef>`,
each entry carrying the call's own `line` and `callee_name`). Like the
others, this is deliberately coarse and heuristic: it can't recognize
JSON parsing hidden behind a wrapper function or a type alias the table
doesn't name. The other 13 languages have no per-language table yet and
so never produce any entries for this marker.

**Regex compile in loop (`regex_compile_in_loop`)** flags a known
regex-compilation call found anywhere inside a loop body — the seventh
slice of the Performance-signal cluster, reusing `io_in_loop`'s `is_loop`
classifier and, like it, implemented for **Rust, Python, and
TypeScript/JavaScript only**. Compiling a regex is orders of magnitude
more expensive than using an already-compiled one, so doing it once per
loop iteration instead of once before the loop is a common, easily-fixed
performance mistake. A small fixed table of regex-construction callee
patterns per language is matched against each call/constructor node's
callee name: Rust's `Regex::new` and Python's `re.compile` both match on
a *qualified* `Type::method`/`module.function` path (reusing each
language's `qualified_call_name` helper, the same one `resource_construction_in_loop`/
`json_parse_in_loop` use), since a bare `new`/`compile` alone is far too
generic — `Vec::new()` and countless unrelated `.compile()` methods
(`ast.compile`, `zlib.compileobj`, etc.) would otherwise match. TypeScript/
JavaScript's `new RegExp(...)` matches on the bare constructor name via
`resource_constructor_callee` instead, mirroring `resource_construction_in_loop`'s
JS classifier — `RegExp` is already distinctive enough there, no
qualified form needed. `repowise-parser::metrics::regex_compiles_in_loops`
reuses the same "currently inside a loop" tracking shape as the other
loop-body markers (`Symbol::regex_compile_in_loop: Vec<RegexCompileInLoopRef>`,
each entry carrying the call's own `line` and `callee_name`). Note this
marker is the reason `resource_construction_in_loop`'s own table
deliberately excludes `Regex::new`/`re.compile`/`new RegExp` — without
that exclusion the same call would double-flag under both markers. Like
the others, this is deliberately coarse and heuristic: it can't
recognize regex compilation hidden behind a wrapper function or a type
alias the table doesn't name. The other 13 languages have no
per-language table yet and so never produce any entries for this marker.

**Nested loop with I/O (`nested_loop_with_io`)** flags an I/O-shaped
call found at loop-nesting depth 2 or deeper — the eighth and final
currently-implemented slice of the Performance-signal cluster, and the
odd one out shape-wise: every marker above only cares whether *some*
loop encloses a call at all, while this one specifically distinguishes
a call inside one loop from a call inside a loop nested inside another
loop, since the latter is potentially O(n²) (or worse) I/O calls rather
than O(n) — the same I/O call at depth 1 is already covered by
`io_in_loop` alone, and staying at depth 1 is not itself a problem this
marker cares about. Implemented for **Rust, Python, and
TypeScript/JavaScript only**, reusing `io_in_loop`'s own `is_loop`
classifier and I/O-callee-name table unchanged — no new pattern
recognition was needed, only a different walk. That walk is a new
`repowise-parser::metrics::matches_in_nested_loops`, structurally
parallel to `matches_in_loops` but tracking a running loop-nesting
*depth* (incremented on entering each loop node, never decremented)
instead of a single in-loop boolean, and only reporting a match once
depth reaches a caller-supplied minimum (2, here) — a shared building
block the other seven loop-body markers don't need, since none of them
care how many loops deep a call is, only whether it's inside one at
all. `repowise-parser::metrics::ios_in_nested_loops` wraps that walk the
same way `calls_in_loops` wraps the plain version
(`Symbol::nested_loop_with_io: Vec<NestedLoopWithIoRef>`, each entry
carrying the call's own `line` and `callee_name`). A call flagged here
is *also* flagged in `io_in_loop` — this is deliberately a depth-2+
subset of that marker, not an independent detection pass, so the two
markers double-count the same call on purpose: the outer one measures
"any loop-body I/O", the inner one measures the specifically worse
nested case, and its default penalty (−0.6) is double the flat −0.3
every other loop-body marker uses to reflect that added severity. The
other 13 languages have no per-language table yet and so never produce
any entries for this marker.

**Nested loop quadratic (`nested_loop_quadratic`)** flags an inner loop
iterating the *same collection* as an enclosing loop — the classic
accidental all-pairs O(n²) scan (`for x in items { for y in items { .. } }`),
usually replaceable with a set/map lookup. Implemented for **Rust,
Python, and TypeScript/JavaScript only**. Where `nested_loop_with_io`
above cares only how deep a call sits and what it calls, this one
compares the two loops' *iterable expressions* and ignores the body
entirely, so the two are complementary rather than overlapping: a
cross-product over two genuinely different collections is not flagged
here, and an all-pairs scan doing no I/O is not flagged there.

Each language contributes a `loop_iterable` normalizer that reduces a
loop's iterable expression to a base collection name, and
`repowise-parser::metrics::quadratic_loop_nestings` walks carrying a
stack of the enclosing loops' normalized names, reporting any loop whose
own name is already on that stack
(`Symbol::nested_loop_quadratic: Vec<NestedLoopQuadraticRef>`, each entry
carrying the inner loop's own `line` and the shared collection's name).
Normalization peels only wrappers that yield the *same* underlying
collection — `items`, `&items`, and `items.iter()` (Rust);
`enumerate(items)`/`sorted(items)` and `d.values()` (Python);
`items.values()` and `Object.keys(items)` (TS/JS) — so a genuinely
narrower sequence (`items.filter(..)`, a comprehension) doesn't
normalize at all and never compares equal to anything. JS's
`Object.keys(x)` deliberately resolves to `x` rather than the `Object`
global, without which every such loop would collapse to the name
`Object` and two unrelated collections would falsely match.

Ranges (`for i in 0..n`, `range(n)`, and C-style `for (let i = 0; ...)`)
are deliberately excluded even though a doubly-nested one is also
quadratic: that shape is usually a deliberate, irreducible grid/matrix
traversal, whereas iterating one *collection* twice is the accidental
scan this marker is after. Its default penalty (−0.6) matches
`nested_loop_with_io`'s — both flag a quadratic-complexity *shape*
rather than a single expensive call, a tier above the flat −0.3
per-occurrence loop-body markers. The other 13 languages have no
per-language normalizer yet and so never produce any entries for this
marker.

**Serial await in loop (`serial_await_in_loop`)** flags an awaited
async call inside a loop body — each iteration blocks on the previous
one, turning what could be one concurrent batch into N sequential
round-trips. Implemented for **Rust, Python, and TypeScript/JavaScript
only** (the three parsed languages whose grammars carry async syntax),
reusing `io_in_loop`'s `is_loop` classifier and the shared
`matches_in_loops` walk unchanged — only the classifier is new.

Each language contributes an `awaited_callee` extractor that recognizes
its await node (`await_expression` in Rust and TS/JS, `await` in Python)
and returns the awaited call's callee name. Two deliberate narrowings:

- **Only calls are flagged.** An await whose operand isn't a call
  (`await somePromise` on an already-created future) isn't reported —
  the issue describes "each iteration's async call", and requiring a
  call is what lets every finding name the thing being awaited.
- **Awaits of concurrency combinators are excluded** — `Promise.all`/
  `allSettled`/`race`/`any` (TS/JS), `join_all`/`try_join_all`/`join`/
  `try_join`/`select_all` (Rust), `gather`/`as_completed` (Python).
  Those are precisely the *fix* this marker points at, so awaiting one
  inside a loop is the deliberate chunked-concurrency shape
  (`for chunk in chunks { await Promise.all(chunk.map(..)) }`), not the
  serial one. Flagging it would punish the correct pattern. TS/JS
  matches the combinator on its qualified `Promise.all` form rather than
  a bare `all`/`race`, which would be far too generic; Python matches
  the bare `gather`/`as_completed` (distinctive enough on their own) but
  deliberately leaves out `asyncio.wait`, since a bare `wait` is not —
  missing it costs at most a false positive, never a false negative.

Its default penalty (−0.3) matches the other per-occurrence loop-body
markers rather than the −0.6 quadratic-*shape* tier: one serialized
await is one extra round-trip, not an order-of-magnitude blowup. The
other 13 languages have no per-language extractor yet and so never
produce any entries for this marker.

**pandas concat in loop (`pd_concat_in_loop`)** flags a
`pd.concat(..)`/`pandas.concat(..)` call inside a loop body — the
accumulate-one-row-at-a-time shape that pandas' own docs call out as an
anti-pattern. Each call reallocates and copies the whole growing
DataFrame, so a single occurrence makes the enclosing loop quadratic in
the number of rows; the fix is to collect rows in a list and
concatenate once after the loop. **Python only**, since pandas has no
equivalent in this port's other supported languages — the other 15
parsed languages always produce an empty list for this marker. Reuses
Python's `is_loop` classifier and `qualified_call_name` helper
unchanged.

The table deliberately covers only `pd.concat`/`pandas.concat` and
**not** a bare `.append(..)`, even though the marker's originating issue
names `DataFrame.append` too. Without type information this port cannot
tell `DataFrame.append` from `list.append` — and appending to a *list*
inside a loop is precisely the fix this marker recommends, so flagging
bare `.append` would penalize the correct pattern far more often than
the wrong one. (`DataFrame.append` was also deprecated in pandas 1.4 and
removed in 2.0, so its real-world incidence is shrinking regardless.) Its
default penalty (−0.6) sits in the quadratic-*shape* tier alongside
`nested_loop_with_io`/`nested_loop_quadratic` rather than the flat −0.3
per-occurrence tier, because one such call is an order-of-magnitude
blowup rather than one extra unit of work.

**Blocking sync in async (`blocking_sync_in_async`)** flags a known
blocking, synchronous call inside an `async fn`/`async def` body — a
blocking call on an async executor's worker thread stalls the whole
reactor, silently degrading every *other* task sharing that thread.
**Rust and Python only**, the two languages the marker's issue scoped it
to as having the clearest async-function AST node and a fixed set of
well-known blocking stdlib calls.

This is the one marker in the Performance-signal cluster whose context
is the enclosing **function** rather than an enclosing loop, so it
shares none of the `is_loop` machinery. Instead each language gets an
`is_async_fn` classifier — Rust checks for a `function_modifiers` child
containing `async`; Python checks for the anonymous `async` token child
that tree-sitter-python emits on an `async def` — and the extractor only
scans a body at all when that returns true. The scan itself is a new
`repowise-parser::metrics::matches_in_body`, the non-loop counterpart to
`matches_in_loops`: same nested-function skipping, no loop tracking.

Tables are matched on the qualified two-segment path (`thread::sleep`,
`fs::read_to_string` for Rust; `time.sleep`, `requests.get`,
`subprocess.run`, `os.system` for Python), since a bare
`sleep`/`read`/`get` would be far too generic. Python additionally
accepts a *bare identifier* callee for `open`, which is distinctive
enough as a builtin on its own — a call written as a method (`fh.open()`)
yields the qualified form and so never matches that entry.

One correctness detail worth naming: in Rust, `tokio::fs::read_to_string`
and `std::fs::read_to_string` reduce to the *same* two-segment path, and
the async one must not be flagged. A call that is itself being `.await`ed
is therefore never reported — being awaited is the only local evidence
distinguishing the async variant from the blocking one. Covered by a
test.

Its default penalty (−0.6) is elevated above the flat −0.3
per-occurrence tier for a different reason than the quadratic-shape
markers: the cost lands on every other task sharing the executor, not
just on the function containing the call. The other 14 parsed languages
have no `is_async_fn` classifier yet and so never produce entries here.

**Blocking I/O under lock (`blocking_io_under_lock`)** flags an
I/O-shaped call — the same table `io_in_loop` uses — made while a
mutex/lock is held. I/O under a lock serializes every *other* thread
waiting on that lock behind however long the I/O takes, turning an
in-memory critical section into a hidden throughput bottleneck.
**Rust and Python only**, the two languages the marker's issue scoped it
to.

The two languages need genuinely different lock-scope shapes, so this
marker ships two extractors rather than one:

- **Rust** is structural. A `let guard = m.lock().unwrap();` binding
  holds the guard until the end of its enclosing block, so every
  statement *after* that binding is inside the critical section. A new
  `repowise-parser::metrics::matches_after_scope_marker` models exactly
  that: the "in scope" flag propagates down into children but only turns
  on for *subsequent* siblings, so each node is visited once and a
  nested block with its own guard can't double-report calls the outer
  scope already covered. It reuses `is_lock_call`'s table and so
  inherits its deliberate exclusion of `RwLock::read`/`write`.
- **Python** is delimited: `with lock:` gives an explicit block, scanned
  via `matches_in_body`.

**The Python side is a name-based heuristic, deliberately.** Python's
`with` is generic, and `lock_in_loop` already documents the same
limitation from the other direction: without type information there is
no way to distinguish a lock context manager from a file handle or a
database transaction. Rather than drop Python support entirely (the
issue asks for it), this matches when the context expression's own name
looks like a lock — `with lock:`, `with self._write_lock:`,
`with mutex:`, `with threading.Lock():`. It will miss a lock bound to an
unconventional name, and could in principle fire on a non-lock that
happens to be named one; a test pins that a plain `with cm:` stays
quiet. Rust's side needs no such guess.

Its default penalty (−0.6) matches `blocking_sync_in_async`'s and for
the same reason: the cost lands on threads *other* than the one running
the call. The other 14 parsed languages have no lock-scope extractor yet
and so never produce entries here.

**Array spread in reduce (`array_spread_in_reduce`)** flags a
`.reduce(..)`/`.reduceRight(..)` callback that builds its result with
array spread — `(acc, x) => [...acc, x]` — instead of mutating and
returning the accumulator. The spread copies the *entire* accumulator on
every step, so a linear fold becomes quadratic. It's a subtle trap
precisely because the spread reads as idiomatic, immutable-style JS and
gives no visual signal that it's doing a full copy each iteration.
**TypeScript/JavaScript only** — this targets the JS array method, which
has no equivalent in this port's other languages.

Detection is self-contained in the `reduce` call rather than depending
on any enclosing loop or function context: find a `call_expression`
whose callee property is `reduce`/`reduceRight`, take its first
argument as the callback, read that callback's first parameter as the
accumulator, then check whether the callback's returned expression is an
array literal containing a `spread_element` of that same accumulator.
Both callback body forms are handled — the expression-bodied arrow
(`=> [...acc, x]`) and a block body with a `return`.

Two deliberate limits: only *top-level* returns in a block body are
considered, so a `return` nested inside an `if` isn't found; and the
spread must name the callback's own accumulator parameter, so spreading
some unrelated array is not flagged. Both err toward under-reporting
rather than guessing. The mutate-and-return form (`acc.push(x); return
acc`) is the recommended fix and never matches — a test pins that, along
with a plain scalar fold (`(acc, x) => acc + x`), which returns no array
at all. Its default penalty (−0.6) sits in the quadratic-*shape* tier.

**SQL cartesian join (`sql_cartesian_join`)** flags a SQL query string
that lists several comma-joined tables without enough join predicates to
connect them — an accidental cartesian product returning `n × m` rows.
That's a correctness bug as much as a performance one, and it's the kind
that surfaces in production rather than review. Implemented for **Rust,
Python, and TypeScript/JavaScript** — the scan reads string-literal
*contents*, so the SQL logic itself is language-agnostic and each
language contributes only a three-line extractor for its own literal
node kinds (Rust's `string_literal`/`raw_string_literal`, Python's
`string`, JS/TS's `string`/`template_string`).

The heuristic, deliberately coarse and **not** a SQL parse — the same
framing as `repowise_workspace::contracts`' route-pattern table: take
the `FROM` clause up to the next clause keyword, split it on commas, and
require one qualified `a.b = c.d` equality predicate in the `WHERE`
clause per additional table (`n` tables need `n − 1` predicates). Both
sides of a predicate must be qualified, which is what separates a join
condition from a plain column filter like `o.status = 1` — a test pins
that such a filter does *not* count toward connecting tables.

Three honest limits: a `FROM` clause containing an explicit `JOIN` is
skipped entirely (its `ON` predicate is a different shape, and the
explicit form is rarely the accidental case); a query assembled by
string concatenation is invisible, since only one literal is ever in
hand; and table aliases are read as the first whitespace-delimited token,
so unusual formatting can confuse the table list. Its default penalty
(−0.6) sits in the quadratic-*shape* tier rather than above it —
arguably the most severe marker in the cluster, but also the most
heuristic, so it doesn't get its own tier.

**Defer in loop (`defer_in_loop`)** flags a Go `defer` statement inside a
loop body. Go runs deferred calls when the enclosing *function* returns,
not at the end of the iteration that queued them, so `defer f.Close()`
inside a loop over ten thousand paths holds ten thousand file handles
open until the loop — and the rest of the function — has finished. It's
one of Go's best-known footguns, and it looks completely correct
locally: the same line one directory up in the function body is the
right thing to write.

**Go only**, and unavoidably so: no other language in this port has a
defer-to-function-exit construct at all, so there is nothing to detect
elsewhere. This is also the marker that gave Go its first per-language
classifier — Go sat outside the whole Performance-signal cluster until
now, and only needed an `is_loop` arm (Go has exactly one loop keyword,
so the C-style, condition-only, infinite, and `range` forms are all a
single `for_statement` node) plus a `defer_statement` callee extractor.
There is no callee-name table here, unlike most of the cluster: the
`defer` keyword *is* the whole signal, so the finding names the deferred
call (`Close`, `Unlock`) purely to make itself actionable.

**A `func` literal ends the defer scope**, and getting that right is what
keeps the marker from penalizing its own remedy. A `defer` runs when the
innermost enclosing *function* returns, and a function literal is such a
function even though it gets no `Symbol` of its own — so wrapping a loop
body in `func() {...}()`, the fix this marker recommends, means the
`defer` now runs at the end of that iteration and is correctly not
flagged. The same rule covers the extremely common `defer wg.Done()`
inside a goroutine's literal. The walk still recurses *into* literals, so
a loop nested inside one keeps its defers flagged; only the
enclosing-loop state is dropped at the boundary. Tests pin both halves.

Its default penalty (−0.6) sits in the heavier tier for the same reason
`blocking_io_under_lock` does — the cost doesn't land on the flagged
line. A single `defer` in a loop fires the marker once but leaks one
resource *per iteration*, so the damage scales with the loop's trip
count rather than with how often the marker matches; counting it
linearly would understate it badly.

**Goroutine in unbounded loop (`goroutine_in_unbounded_loop`)** flags a
Go `go` statement inside a loop body whose only limit on concurrency
would be the iteration count. One goroutine per input item works fine in
testing with a handful of items and fails only in production at scale,
where it exhausts memory locally or overwhelms whatever the goroutines
call.

**The "unbounded" qualifier is the whole design.** A loop is treated as
bounded when its body contains a channel send (`sem <- struct{}{}`) or
receive (`<-tokens`) — the acquire half of Go's standard semaphore and
worker-pool idioms, and the only mechanism recognized here.
`sync.WaitGroup` deliberately does *not* suppress: `wg.Add`/`wg.Wait`
bound how *completion* is tracked, not how many goroutines run at once,
so a `WaitGroup` fan-out stays flagged. A test pins both halves.

The bound scan **skips the launched goroutine's own subtree**, which is
what keeps the marker useful. In `go func() { results <- work(v) }()`
the channel send is the goroutine reporting its result, not the loop
throttling how many exist; counting it would suppress precisely the case
worth flagging. A test pins that too. Suppression is also scoped per
loop and inherited inward — an inner loop can't un-bound the semaphore
an enclosing loop already acquired — which is why this marker needs its
own walk rather than reusing the `matches_in_loops` "am I inside a loop"
boolean the rest of the cluster shares.

The inline `go func() {...}()` form has no callee name to report, so it
shows as `func literal` rather than being dropped — it's by far the most
common shape, and dropping it would blind the marker to it. Its default
penalty (−0.6) sits in the heavier tier for the same reason
`defer_in_loop` does: one statement fires the marker once but spawns a
goroutine per iteration.

**Membership test in loop (`membership_test_in_loop`)** flags an
`x in xs` / `xs.contains(&x)` / `xs.includes(x)` test inside a loop body
where `xs` is a list. Each test scans the whole list, so running one per
iteration makes the loop O(n × m) while it reads as linear; building a
set once outside the loop makes each test O(1).

**Telling a list from a set is the entire difficulty**, and issue #182
flagged it as needing a design pass because this port tracks no type
information. Rather than build type inference, this takes the narrow
slice that's reliably visible in one function's own text: a **local
binding whose initializer shape names the collection kind outright**. A
first pass over the function body collects those into a name → kind map
(`xs = [..]`/`sorted(..)` and `let xs = vec![..]` and `const xs = [..]`
are lists; `{..}`/`set(..)`/`HashSet::new()`/`new Set(..)` are not), and
the loop walk then only flags a test whose target resolves to a list.

**Everything else is deliberately left unflagged.** A parameter, a
struct field, an imported constant, or any initializer whose shape
doesn't settle the question never enters the map and is silently
skipped. That makes this a low-recall, high-precision marker on
purpose — a false positive here tells someone to "fix" a lookup that was
already O(1), which is worse than staying quiet. A name rebound to a
different kind (`xs = [..]` then `xs = set(xs)`) is demoted for the same
reason: one pass can't know which binding is live at the test site.

Rust is where the binding map earns its keep: `Vec::contains` and
`HashSet::contains` are spelled identically at the call site, so nothing
local distinguishes the O(n) scan from the O(1) lookup. There the
*declared type* wins over the initializer when both are present, which
is what makes `let seen: HashSet<_> = xs.into_iter().collect();`
resolvable when `.collect()` alone says nothing. In JS/TS, `Set.has` is
simply never a membership target — it's already the form this marker
recommends — and strings, which share `includes`/`indexOf` with arrays,
fall out naturally because a string binding never resolves to a list.

Its default penalty (−0.6) sits in the quadratic-*shape* tier alongside
`nested_loop_quadratic`. Tests pin the list-vs-set split in all three
languages, plus `not in`, inline list literals, unknown bindings, and
rebinding.

**Hot-path sync I/O (`hot_path_sync_io`)** is the only marker in this
crate built from **two independent signals**: a structural one (a
blocking file/network call is present in the function) and an empirical
one (git says this file is a hotspot — high churn × complexity).
Neither alone is a finding. A blocking read in a rarely-run setup path
is fine, and a hotspot with no blocking I/O has nothing to fix here; it's
the intersection that's worth reporting, and it's a much
higher-precision flag than either half would be.

**How the git data gets in without compromising the crate.**
`repowise-health` is deliberately a pure function of the index and the
call graph — it knows nothing about git, and adding a git dependency to
get one marker would be a bad trade. Instead the *caller* computes which
files are hot and passes them in as plain paths, via a new entry point:

```rust
analyze_with_hotspots(&index, &graph, &weights, &hot_files)
```

`analyze`/`analyze_with_weights` still exist and now delegate with an
empty set, so every existing caller (docs, dashboard, MCP, server)
compiles and behaves exactly as before and simply never sees this
marker. Only `repowise health` currently supplies the set.

"Hot" is a **relative rank**, not an absolute score: hotspot scores are
churn × complexity and so aren't comparable between repos — "the files
this repo churns hardest" travels, "score above 500" doesn't. The set is
the top 10 files, *further capped to a quarter of the repo*, excluding
anything scoring zero. That second cap matters more than it looks: a
plain top-10 makes every file in a small repo hot, which silently
degrades this into "any sync I/O anywhere" and throws away the empirical
half of the signal that justifies the marker in the first place. (A
two-file repo gets a top 1; the 10-file bound only starts binding at 40+
files.)

If git history isn't available — no repo, a shallow clone, git missing —
the set is empty and the marker silently doesn't fire. Losing one marker
is a far better outcome than refusing to score the codebase.

The call-recognition half reuses `io_in_loop`'s callee table verbatim;
what changes is only the scope, a whole-body scan rather than a
loop-body one. Its default penalty (−0.3) sits in the per-occurrence
tier alongside `io_in_loop`: an individual blocking call is a real but
bounded cost that doesn't worsen with input size. The precision here
comes from the hotspot gate, not from a heavier weight.

**Near-duplicate code (`dry_violation`)** catches *partial* duplicates
the exact-hash `Duplicate code` marker misses entirely — a function
that's mostly identical to another with a few renamed variables or a
tweaked constant, where even one differing character breaks a hash
match. Rather than growing `Symbol`/`FileRecord` with raw body text just
for this, it re-reads each candidate symbol's source fresh from disk
(the same tradeoff `get_symbol` and the ADR code-comment/inline-marker
sources already make elsewhere in this port) and tokenizes it
(identifier/number runs plus single-character punctuation), then splits
each symbol's token sequence into overlapping 3-token windows, hashed
with an incremental Rabin-Karp rolling hash. Windowing over *tokens*
rather than raw characters matters because a renamed identifier changes
length — `total` -> `sum` shifts every subsequent character position,
which would misalign every raw-character window from that point on even
though the code is otherwise identical; a token-level window only loses
the windows actually touching the renamed token. Two symbols become a
candidate pair the moment they share one window hash (avoiding
brute-force all-pairs comparison), then are flagged once their shared
window count reaches 50% of the smaller symbol's total — pairs already
caught by the exact-hash `Duplicate code` marker (identical `body_hash`)
are explicitly excluded so a pair is never reported under both finding
kinds at once.

Deferred markers from the original repowise (not implemented): the
ML-calibrated organizational-signal markers (`churn_risk`,
`co_change_scatter`, etc. — see issue #62, a design-level "needs-human"
question, not a mechanical gap). Hotspots and bug-fix history are now
implemented (see "Git analytics" below) but aren't yet folded into the
health score itself — that's a natural follow-up, not done here.

## Exporting wiki pages

`repowise export --out <DIR>` copies the pages generated by `repowise docs`
out of `.repowise/wiki/` into a target directory, preserving the tree. That
makes them publishable — a docs site, a PR artifact, an attachment to a review
— rather than usable only in place or through the dashboard.

**A non-empty target is refused unless you pass `--force`.** An export target
is often something like `./docs`, and quietly merging into (or partly
overwriting) a hand-written docs tree would be destructive and awkward to undo.
With `--force`, unrelated files in the target are still left alone; only pages
are written.

**An empty result is an error, not a silent success.** If `repowise docs` was
never run, or the wiki exists but holds no pages, `export` says so and exits
non-zero — reporting "exported 0 pages" successfully would be
indistinguishable from a real export of a repo that genuinely has no docs.

### Architecture model (`--format json-graph`)

`repowise export --out <DIR> --format json-graph` writes the dependency graph
to `<DIR>/architecture.json` in [JSON Graph Format](https://jsongraphformat.info/).

**Why JGF over DOT or Mermaid.** It's the only one of the three that carries
per-node metadata losslessly. This graph's nodes know their language, line
count, symbol kind, complexity, nesting depth, and parent type — DOT and
Mermaid would have to discard all of it to render a picture. JGF keeps it, so
the export is something a tool can consume rather than only something a human
can look at.

File nodes are `file:<repo-relative-path>`, symbol nodes are
`symbol:<path>::<name>@<line>`, and edges carry `contains`/`imports`/`calls`
relations. Paths are repo-relative with forward slashes, and symbol ids are
rebuilt from the relative path rather than reusing `Symbol::id` (which embeds
an absolute path) — so an export is portable between machines. Output is
deterministic, so two exports of an unchanged repo are byte-identical and
therefore diffable.

**The graph is partial, and says so.** This port resolves imports and calls
with directory-layout heuristics, not compiler-grade name resolution, and
leaves ambiguous or external references unresolved rather than guessing. Those
have no target node, so they have no edge. Rather than emit a graph that merely
*looks* complete, `graph.metadata.unresolved` reports the counts and names the
import stems that failed to resolve, and the command prints the same warning to
the terminal. On this repo that's 381 unresolved imports and 7804 unresolved
calls — a reader drawing conclusions from the edges needs to know that.

## Coverage health markers

Once coverage is ingested, `repowise health` reports two more markers:

| Marker | Fires when | Penalty |
| --- | --- | --- |
| `coverage-gap` | a measured file is under 50% line coverage | −0.4 |
| `untested-hotspot` | a churn hotspot **and** ≥4 dependents **and** <40% coverage | −1.0 |

`untested_hotspot` is the heavier of the two because it needs **three
independent signals to agree**. Each alone is unremarkable — a well-tested
hotspot is fine, an untested leaf nobody imports is fine — but the
intersection is where risk actually concentrates. That's the same reasoning
behind `hot_path_sync_io`, and it's why this can carry a real weight without
flooding scores with false positives. A file that earns `untested_hotspot`
does **not** also get charged `coverage_gap`; stacking both would
double-penalize one underlying fact.

**Without ingested coverage, neither marker ever fires** — a repo that never
ran `coverage add` is not reported as untested. That guarantee runs all the
way down to `line_coverage_of`, which returns `None` for an unmeasured file
and `Some(0.0)` only for one measured with nothing executed.

Coverage reaches `repowise-health` through a new `analyze_with_context` entry
point, following the precedent `analyze_with_hotspots` set for git data: the
crate stays a pure function of its inputs, and `analyze`/`analyze_with_weights`
behave exactly as before. Both penalties are configurable via `--weights` like
every other marker.

These are ordinary **fixed-penalty** markers — nothing here presumes an answer
to the ML-calibrated-weights question in issue #62, which stays open.

## Test coverage

`repowise coverage add <REPORTS...>` ingests LCOV reports; `repowise coverage
status` summarises them. This is the foundation of the test-intelligence layer
— the reference's `impacted-tests` and its `untested_hotspot`/`coverage_gap`
health markers both build on it (tracked in issues #242 and #243).

Two data shapes are stored, matching the reference:

- a **per-file aggregate** merged across every test, which drives coverage-gap
  analysis; and
- a **per-test map** (from LCOV's `TN:` records) of which test executed which
  lines, which is what `impacted-tests` will need.

**LCOV only, deliberately.** The reference also reads Cobertura XML, Clover
XML, coverage.py's SQLite files, and its own normalized JSON. LCOV is the one
format parseable with no new dependency; every other would pull in an XML or
SQLite crate. The rest are follow-ups.

**Reports merge rather than replace** (`--replace` opts out), so a suite split
across CI shards can be ingested one report at a time.

Two distinctions the implementation is careful about:

- **"Never measured" is not "0% covered."** `line_coverage_of` returns `None`
  for a file no report mentions and `Some(0.0)` for one that was measured and
  had nothing run. Collapsing those would report unmeasured files as untested.
- **Paths that match nothing are reported loudly, not dropped.** LCOV paths may
  be absolute from whatever machine ran the suite, so ingest resolves them
  against the repo root and retries by longest suffix — a CI path like
  `/builds/proj/src/lib.rs` still finds the local `src/lib.rs`. Anything that
  still doesn't resolve is warned about by count and by name, because coverage
  that silently matched nothing looks identical to coverage that worked.

## Impacted tests

`repowise impacted-tests [REVSPEC] [PATH]` intersects a diff's changed lines
with the per-test coverage map, printing the tests that provably execute those
lines. Needs a prior `repowise coverage add` with reports carrying `TN:`
records.

**An empty list is never presented as reassurance.** Four distinct situations
produce no tests, and only one of them means "no test is affected":

| Status | Meaning |
| --- | --- |
| `per-test map consulted` | genuine — no test executes the changed lines |
| `no coverage data ingested` | can't answer; nothing was ever measured |
| `coverage present, but no per-test map` | can't answer; reports had no `TN:` records |
| `no changed lines in any measured file` | can't answer; diff touched only unmeasured files |

The three "can't answer" states print `CANNOT ANSWER` and say so in words. This
matters more than it might look: a developer who reads "no impacted tests" as
"safe to skip testing", when the real cause was that no coverage was ever
ingested, has been actively misled into shipping untested code. Even the
genuine empty result says it means untested *by the ingested suite*, not safe.

An entirely empty diff is called out separately, naming the likely cause:
`git show` reports no diff for a **merge commit**, which otherwise reads as
"this change touched nothing".

Line ranges come from `git diff -U0` hunk headers. Deletion-only hunks
contribute nothing, which is correct — a coverage map records lines that exist
and ran, and a deleted line does neither.

## Doctor

`repowise doctor [PATH]` runs setup diagnostics and reports each as
`pass`/`warn`/`FAIL` with a one-line remedy for anything not passing. It's
**diagnostic only** — it never mutates state.

It exists because this port has many environment-dependent, degrade-softly
paths, and each one previously only revealed itself when you happened to run
the command that needed it. Checked: the `git` binary, whether the directory
is a git repo, **whether the clone is shallow**, index presence, and the two
optional env vars (`REPOWISE_GITHUB_TOKEN`, `REPOWISE_LLM_BASE_URL`) along
with exactly what each degrades to when unset.

**A degraded-but-working setup is a `warn`, never a `FAIL`,** and only a
hard failure produces a nonzero exit. Missing an optional token is not an
error; reporting it as one would train people to ignore the command — and
make `doctor` useless in a CI gate that only cares about real breakage.

The shallow-clone check is the one most worth having: a shallow clone doesn't
make `hotspots`/`coupled` fail, it makes them *quietly under-report*, which is
far harder to notice. That check is skipped entirely outside a git repo rather
than reporting a misleading "full history".

## Auto-sync (post-commit hook)

`repowise hook install` writes a `post-commit` hook into `.git/hooks` that
refreshes the index after each commit; `uninstall` removes it and `status`
reports which of three states you're in.

The reference repowise drives auto-sync five ways (post-commit hook, file
watcher, GitHub webhook, GitLab webhook, polling). Only the hook is
implemented here, deliberately: it's the one mechanism needing **no new
dependency, no daemon, and no server**. `repowise watch` would require a
filesystem-notification crate and is not implemented.

**The hook runs `repowise update` detached and silenced.** Git waits for
`post-commit` to exit, so a slow re-index would stall every commit — which
would be worse than having no auto-sync at all.

**A `post-commit` hook this tool didn't write is never touched.** That path is
somewhere users and other tools legitimately put things, so the hook body
carries a marker line, and only its presence authorizes an overwrite or a
delete. Anything else is reported as `foreign` and left byte-for-byte alone —
`install` and `uninstall` both refuse rather than clobber. Re-installing our
own hook is idempotent.

A worktree or submodule (where `.git` is a *file*, not a directory) is
reported as a clear error rather than silently creating a `.git/hooks`
directory that git will never consult.

## Index status

`repowise status [PATH]` reports whether the index still describes the tree
on disk — deliberately distinct from `repowise overview`, which reports what's
*in* the index. Shows how many indexed files have been modified or deleted
since indexing, plus whether wiki pages and a dashboard have been generated.
`--verbose` lists the individual files instead of only counting them.

Staleness is checked by comparing each indexed file's mtime against the
index's own, **not** by diffing against git. That works in a repo with no git
history, a shallow clone, or no git at all, and it catches uncommitted edits
that a diff against the indexed commit would miss.

The tradeoff is stated in the command's own output rather than left implicit:
files *created* since indexing aren't detected, because finding those needs
the full re-walk `repowise update` already does. A freshness check that
quietly missed new files would be worse than one that says so.

## Change risk

`repowise risk [REVSPEC] [PATH]` scores the diff shape of one commit or a
`base..head` range, defaulting to `HEAD`. It's a CLI surface over the same
`repowise_git::change_risk` that backs the `get_change_risk` MCP tool — one
scoring path, not two.

Reported: the 0–10 score with a `low`/`moderate`/`high` band, the diff shape
(files, lines added/deleted, subsystems touched), the concentration entropy
(0.00 = the change sits in one file, 1.00 = spread evenly across all of
them), and the head commit's author with their prior-commit count in the repo.

**The band is presentational only.** The underlying score is a documented
fixed-weight heuristic over diff shape — deliberately *not* the reference
repowise's ML-calibrated model (see issue #62 for why that stays an open
question here). Treat it as a rough approximation, not a calibrated
probability, and read `repowise-git`'s `change_risk` docs for the formula and
its saturation points.

## Dead-code detection

`repowise dead-code [PATH]` lists confidence-tiered dead-code candidates:
functions/methods with zero resolved in-repo callers. It's the same
analysis (`repowise_health::find_dead_code`) that backs the
`get_dead_code` MCP tool and the dashboard's dead-code view — this is a
CLI surface over it, not a second implementation, so the tiers and risk
factors are exactly the ones documented under "MCP server" below.

- `--min-confidence <low|medium|high>` (default `low`) filters to that
  tier and above, mirroring the MCP tool's argument of the same name.
- `--limit <N>` (default 50) caps the listing; the full matching count is
  still reported, so a truncated list never reads as a complete one.

**Confidence is a claim about this port's static call graph, not about
runtime safety** — and in particular the analysis does not exclude
`#[test]` functions, which have no in-repo callers by construction and so
dominate the `high` tier on a well-tested codebase. Treat the output as a
list to review, not a list to delete. Excluding test functions would
change `get_dead_code`'s results too, so it's deliberately not done here.

## Git analytics

`repowise hotspots`/`ownership`/`coupled` shell out to `git log`/`git
blame` under the hood — no persistence, no caching, just re-run each
time against the repo's real history:

- **Churn**: number of commits touching a file, from a single `git log
  --name-only` walk of the whole history.
- **Hotspot score**: `churn × total cyclomatic complexity` of the file's
  symbols (complexity already computed by `repowise-parser`). `hotspots()`
  ranks files by a **recency-weighted** variant of this score: each commit
  contributes `exp(-age_days / 90)` instead of a flat `1`, so a file with
  the same raw churn as another but touched more recently ranks higher.
  The raw (non-decayed) score is still reported alongside it for
  transparency.
- **Bug-fix commits**: commits whose message contains a fix-like keyword
  (`fix`, `bug`, `hotfix`, `patch`) touching the file, **or** whose message
  references a GitHub issue (`#123`) that's closed with a bug-like label —
  a commit counts if either heuristic matches (a union, not a replacement).
  The keyword check is always on; the linked-issue check is a stronger,
  complementary signal but needs the GitHub API, so it's opt-in behind a
  `REPOWISE_GITHUB_TOKEN` environment variable (mirroring `repowise-adr`'s
  PR-body decision source) and degrades to keyword-only detection when
  there's no token, no GitHub-hosted `origin` remote, or a lookup fails.
  Both heuristics remain heuristics, not ground truth: a fix described
  without a keyword or a linked issue won't be counted, and an unrelated
  commit that happens to mention one will be.
- **Co-change coupling**: files that appear together in the same commit,
  counted per pair. Commits touching more than 50 files are skipped when
  building this (a rename sweep or vendor bump would otherwise flood
  every touched file's coupling list with noise).
- **Ownership**: per-author share of a file's lines from `git blame
  --line-porcelain`.
- **Bus factor**: the smallest number of authors whose combined share
  reaches 50% of a file's lines — how many people would have to leave
  before most of the file has no author left who has touched it. Derived
  from the ownership shares above, so it costs no extra `git` invocation.
  The reference repowise documents a bus factor but doesn't publish its
  threshold; 50% is chosen here as the most defensible round number (a
  simple majority), and a higher bar like 80% would answer the different,
  less actionable question "who wrote nearly all of it". `repowise
  ownership` reports it in words rather than as a bare number, since
  "bus factor: 1" reads to some as "one tidy owner" — the opposite of
  what it means. A file with no blameable lines reports `n/a`, which is
  distinct from a bus factor of 1.

### Structural-tier languages

Objective-C, R, Zig, Julia, Elm, OCaml, Crystal, Nim, and D (issue
#70) get recognized as named languages (`Language::from_extension`)
but have no tree-sitter grammar wired up at all — `repowise_parser::
parse_file` gives them a bare, zero-symbol `FileRecord` (just the
file's path/language/line count) instead of folding them into the
`other_files` count like a genuinely unrecognized extension. That's
enough to make them visible everywhere `RepoIndex.files` drives a
view:

- `repowise overview` reports real per-language file counts (e.g. "R
  3", not lumped into "Other").
- `repowise hotspots` includes them, churn computed from real `git
  log` history — but always at a `0` score, since there are no
  symbols to sum complexity over (git-history signal only, no
  complexity signal, matching repowise's own "Structural tier" naming).
- The dashboard's Symbols/Health/Dead-code views correctly show
  nothing for these files (zero symbols, not an error).

`repowise ownership`/`coupled` (and their `/api/ownership`/
`GET /api/graph` dashboard equivalents) already worked for these
files even before this — both take an explicit file path and read
straight from `git blame`/`git log`, bypassing `RepoIndex` entirely.

## Documentation generation

`repowise docs` renders one markdown page per indexed file under
`.repowise/wiki/<relative-path>.md` (e.g. `crates/foo/src/lib.rs` →
`.repowise/wiki/crates/foo/src/lib.rs.md`), each containing:

- Its symbol list (function/method/class/struct, with parent and line number)
- Resolved dependencies and dependents (from `repowise-graph`)
- Its health findings (from `repowise-health`), or "No findings."

No LLM is involved — every page is rendered from data the other layers
already computed. Freshness is tracked via a hash of the file's own raw
source, embedded as the page's first line and compared against the
previous run's page (if any) to report each page as new/changed/
unchanged. This is a **per-file, own-source-only** signal: a page can be
reported "unchanged" while its actual rendered content differs, if what
changed was cross-file data (a new caller elsewhere, a health finding
driven by another file) rather than this file's own source — pages are
always rewritten with current data regardless of the reported status, so
content is never stale, only the *status label* can undersell how much
changed. Not implemented from the original: the dashboard's doc browser.
LLM-written prose on top of these pages does now exist (`repowise
generate` — see the next section), but as a separate opt-in crate rather
than a `repowise-docs` feature, keeping this crate itself LLM-free.

## LLM-assisted wiki summaries

`repowise generate [PATH]` is opt-in and needs `repowise docs` to have
already been run: it reads each file's existing wiki page under
`.repowise/wiki/`, asks an LLM to write a 2-3 sentence plain-English
summary of that page's already-deterministic content (symbol list,
dependencies, health findings), and inserts it as a "## Summary" section
right after the page's title. Re-running replaces a previous summary
rather than stacking a second one.

This is the first, narrow slice of what was previously a fully-deferred
LLM tier — see `repowise-llm`'s module doc comment for the other three
LLM-dependent features (RAG chat, refactor-plan codegen, doc-gen-as-
decision-source) still deferred as separate follow-ups, since each needs
its own retrieval/context design.

Configuration is three environment variables, all read only at the
`repowise generate` call site (never baked into the index):

- `REPOWISE_LLM_BASE_URL` — the on/off switch. Unset and `repowise
  generate` exits with a message pointing here rather than silently doing
  nothing. Point it at any OpenAI-compatible `/v1/chat/completions`
  endpoint — including a self-hosted
  [`rusty_provider`](https://github.com/baileyrd/rusty_provider) instance
  (a Rust router fronting OpenAI/Anthropic/Gemini/Groq/Together/Fireworks
  behind one URL with config-driven fallback chains).
- `REPOWISE_LLM_MODEL` — a direct `"provider/model"` string or a route
  alias (e.g. `rusty_provider`'s `"smart"`), defaulting to `"smart"`.
- `REPOWISE_LLM_API_KEY` — optional; omit it for an endpoint that doesn't
  require one.

One file's failure (no wiki page yet, a failed LLM call) doesn't stop the
rest — `repowise generate`'s summary line reports written/skipped/failed
counts, and every other file's page is still attempted. `repowise-llm`
uses `ureq` (synchronous), the same HTTP-client choice `repowise-adr`/
`repowise-git`'s own opt-in network calls already made, so this command
doesn't need to pull an async runtime into an otherwise-synchronous CLI
the way `repowise serve` does.

## Architectural decision mining

`repowise decisions` mines six of the original's eight decision sources:

- **`docs/adr/*.md` files**, parsed against this repo's own ADR template
  (`# ADR-XXXX: Title`, then `Status:`/`Date:` lines). An unfilled
  template (title still literally `<Title>`) is skipped rather than mined
  as a real decision.
- **Decision-like commit messages** — a message containing one of a
  19-verb keyword set (`decide`, `decision`, `chose`, `chosen`,
  `switch to`, `adopt`, `instead of`, `migrate`, `replace`, `deprecate`,
  `drop`, `rewrite`, `split`, `revert`, `opt for`, `in favor of`,
  `settle on`, `consolidate`, `standardize on`). A heuristic, not ground
  truth, same framing as the bug-fix-commit detection in git analytics.
- **Decision-like merged PR bodies** — the same keyword heuristic as
  commit messages, applied to a merged PR's title/body via the GitHub
  API. This is the one decision source (and the one place in
  `repowise-adr`) that makes a network call, and only when a
  `REPOWISE_GITHUB_TOKEN` environment variable is set: a local
  codebase-analysis CLI making unsolicited outbound HTTP requests would
  be surprising, so this is opt-in rather than falling back to GitHub's
  unauthenticated (and much more rate-limited) API. No token, no git
  remote, or a remote that isn't GitHub-hosted all degrade to "this
  source found nothing" rather than erroring — same tradeoff the other
  two sources already make for a missing `docs/adr/` or unreadable git
  history. Unlike the other two sources, a PR decision links to the
  files that PR actually touched (already reported by the GitHub API)
  rather than falling back to text-matching.
- **Decision-like code comments** — the same keyword heuristic again,
  applied to the comment/docstring block sitting *directly above* an
  indexed symbol's declaration (`///`/`/** */` above a Rust/Java/
  C-family declaration, `#`-prefixed lines above a Python/Ruby function).
  Pure filesystem/parsing, no new dependency, unlike the PR-body source.
  Deliberately scoped to that one convention — Python/JavaScript's
  alternative of a docstring as the function body's first statement
  isn't handled, a documented gap rather than a silent one. Linked to
  the file the comment sits in, the same "authoritative, not
  text-matched" treatment PR decisions get.
- **Inline decision markers** — a small, explicit tag vocabulary
  (`WHY:`, `DECISION:`, `TRADEOFF:`, `ADR:`, `RATIONALE:`, `REJECTED:`)
  recognized as a prefix inside any comment syntax (`#`, `//`, `/* */`),
  wherever it appears in a file — not tied to sitting above a particular
  symbol the way the code-comment source is. Much lower false-positive
  risk than the freeform code-comment source: this is an explicit opt-in
  convention, not a keyword guess, so every match is deliberate. A plain
  text scan (not language-specific parsing), same "pure filesystem work,
  no new dependency" framing as the code-comment source. Linked to the
  file the marker sits in.
- **Keep-a-changelog-style CHANGELOG sections** — `CHANGELOG.md`/
  `HISTORY.md`/`NEWS.md`/`CHANGES.md` at the repo root (whichever is
  found first, case-insensitive), scanning for `### Changed`/
  `### Removed`/`### Deprecated`/`### Security` section headings (a
  heading-text match, not a full keep-a-changelog spec parser).
  `### Added`/`### Fixed` are deliberately excluded — purely additive or
  bug-fix entries aren't architectural decisions the way a
  change/removal/deprecation/security call generally is. Pure
  filesystem/parsing, no new dependency. Unlike the PR-body/code-comment/
  inline-marker sources, a changelog entry is linked the same way
  ADR-file/commit-message decisions are (text-matched against the
  index) rather than an authoritative self-link to the changelog file:
  the changelog file itself isn't what the decision is *about*, unlike a
  PR's diff or the file a comment sits in.

Each ADR-file/commit-message/changelog decision is linked to the indexed
files it mentions: either the file's own relative path appearing
verbatim in the decision's body text, or one of its non-module symbol
names (4+ characters, to cut down on false positives from short
identifiers) appearing as a whole word. Matching text, not meaning — a
decision that only refers to a file descriptively ("the queue module")
won't be linked. Supersession is read directly from an ADR's
`Status: Superseded by ADR-XXXX` line — no new front-matter convention
was needed since the
existing template already has one.

Not implemented from the original's eight sources: Slack and issue
trackers — this repo doesn't have integrations for either anyway.
Recency/confidence scoring on mined decisions is also not implemented.

## MCP server

`repowise serve [PATH]` runs an MCP server over stdio (via the official
[`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) SDK), requiring
a prior `repowise init`/`update`. Eight tools are implemented:

- **`get_overview`** — the same data as `repowise overview`: file/language/
  symbol counts, edge counts, most-depended-on files.
- **`search_codebase`** — the same substring search as `repowise search`.
- **`get_context`** — a file's symbols, resolved dependencies/dependents,
  and health score/findings in one call. This is the tool that matters
  most for the original's stated goal (cutting an agent's token spend on
  context-loading): one round-trip instead of separate search/deps/health
  reads pieced together by the caller.
- **`get_risk(file?, top_n?)`** — `get_context` plus git-history risk
  data: hotspot score, churn, and bug-fix-commit count from
  `repowise-git`. Given `file`, returns that file's risk profile alone;
  given no `file`, returns the `top_n` riskiest files repo-wide, ranked
  by (recency-weighted) hotspot score. Degrades to zero/empty git data
  (rather than erroring) when the indexed root isn't a git repository —
  same tradeoff `repowise-dashboard`'s hotspots section already makes.
- **`get_change_risk(revspec?)`** — a deterministic 0-10 risk score for a
  single commit or a `base..head` range (defaulting to `HEAD`), computed
  from diff-shape metrics via `git diff`/`git show`/`git rev-list`: lines
  added/deleted, files touched, subsystems (top-level directories)
  touched, change concentration (a normalized Shannon entropy of how
  evenly the diff is spread across files), and the head commit author's
  prior-commit count as an experience proxy. These combine via a fixed,
  documented weighting (0.25 lines, 0.20 each for files/subsystems/
  author, 0.15 concentration) into the final score. **This is not the
  original's tool** — the original feeds the same kind of diff-shape
  metrics into a pre-trained L2-logistic-regression model; this port has
  no labeled defect corpus or training pipeline to reproduce that (see
  issue #42 and the category-A "ML-calibrated scoring" issue), so this is
  a deliberately simple heuristic approximation, not a calibrated
  probability. Errors (rather than degrading to zero) when the indexed
  root isn't a git repository, since there's no diff to compute at all.
- **`get_symbol(symbol_id, context_lines?)`** — a symbol's raw source
  text, sliced from its own file at the `start_line..end_line` span
  `search_codebase`/`get_context` report (both now include each symbol's
  `id`). `context_lines` (default 0) pads that span by the same number of
  lines on each side, clamped to the file's actual bounds rather than
  erroring on an out-of-range request. Re-reads the file fresh from disk
  each call, the same "don't trust the index for content, only for line
  numbers" tradeoff `repowise-docs`'s freshness tracking already makes —
  so edits since the last `init`/`update` are reflected, at the cost of
  the returned span possibly being off if line numbers have since shifted.
- **`get_why(targets?)`** — architectural decisions mined from
  `docs/adr/*.md`, decision-like commit messages, decision-like merged PR
  bodies, decision-like code comments, inline decision markers, and
  keep-a-changelog-style CHANGELOG sections (via `repowise-adr`), the
  same data as `repowise decisions --for-file`. `targets` is a list
  of file paths or symbol ids (mixing both is fine — a symbol id resolves
  to its own file); a decision is returned if its body links to at least
  one target's file. Omit `targets` (or pass an empty list) to get every
  mined decision. A thin wrapper with no new mining logic of its own —
  the same "reuse an existing library call" shape as `get_overview`/
  `search_codebase`.
- **`get_dead_code(min_confidence?, safe_only?, limit?)`** — functions/
  methods with zero resolved in-repo callers (the same base signal as the
  `possibly-dead-code` health marker), tiered `low`/`medium`/`high` by two
  cheap risk factors: an ambiguous same-named symbol elsewhere in the
  repo (a call meant for this one could have resolved to that one
  instead), and an unresolved import elsewhere whose last path segment
  matches this file's stem (something may have meant to import this file
  but this port's heuristics couldn't confirm it). Zero risk factors ->
  `high`; one -> `medium`; both -> `low`. `min_confidence` filters to
  that tier and above; `safe_only` narrows to `high` only — the closest
  this tool gets to the original's "safe to delete" designation, though
  it explicitly is **not** a runtime-safety guarantee at any tier:
  reflection, dynamic dispatch, and entry points are all invisible to
  this port's static call graph, the same caveat the `possibly-dead-code`
  marker already carries. `limit` (default 50) caps the returned list;
  the response's `total_matching` reports how many candidates matched
  before truncation. This is a documented approximation of the
  original's dead-code model (4 finding kinds, 3 confidence tiers, and a
  runtime-load risk factor this port has no way to assess) — see
  `repowise_health::find_dead_code` for the exact tiering logic.

Every call re-loads `.repowise/index.json` and rebuilds the dependency
graph fresh — no caching across calls, consistent with how `hotspots`/
`ownership`/`coupled`/`decisions` already work in this port.
(`get_change_risk` doesn't touch the index at all — it's pure `git`
plumbing, same as `repowise-git`'s other functions.)

Not implemented from the original's ~10 tools: the rest of the original's
tool surface beyond what this port's other layers currently support.

## Dashboard

`repowise dashboard [PATH]` writes one self-contained static HTML page to
`.repowise/dashboard/index.html` — open it directly in a browser, no
server to run. Kept deliberately simple: a single page combining five
sections, each degrading gracefully to an explicit "not available"
placeholder (never a silently blank section) when its data doesn't exist:

- **Overview** — same data as `repowise overview`.
- **Code health** — average score, markers by kind, lowest-scoring files
  (same data as `repowise health`).
- **Hotspots** — top files by churn × complexity (same data as `repowise
  hotspots`), or a placeholder if `PATH` isn't a git repo.
- **Architectural decisions** — mined ADRs/decision-commits (same data as
  `repowise decisions`), or a placeholder if none are found.
- **Symbols** — every indexed symbol (name, kind, file, line), with a
  small embedded-JS dropdown that filters the table by kind
  (function/method/class/etc.) client-side. No external requests and no
  build step: the whole table is embedded once in the page, and the
  dropdown just toggles row visibility — the only JS in the dashboard.

Every file path rendered in the overview/health/hotspots/symbols tables
above is a **drill-down link** to that file's `repowise-docs` wiki page
(`.repowise/wiki/<path>.md`) when one already exists on disk — `dashboard`
checks for it directly rather than generating wiki pages itself (that
would duplicate `repowise-docs`'s own freshness tracking and re-read every
file from disk on every dashboard build, even when nothing changed). Run
`repowise docs` before `repowise dashboard` to get working drill-down
links; without it, file paths render as plain text rather than a broken
link. The Architectural decisions table isn't linked this way since its
rows are decisions, not files (its "Linked files" column is just a count).

Regenerating means re-running the command — there's no auto-refresh, and
this static page has no live search either. A genuinely live version is
now underway (`repowise serve-dashboard`, see the next section); this
static page isn't going away in the meantime, since it needs nothing
beyond the CLI to generate and view.

## Live dashboard server

`repowise serve-dashboard [PATH]` starts a real, long-running server —
`repowise-server` (axum) — rather than writing a static file: pivoting the
dashboard to genuine parity with repowise's own Next.js-frontend/
FastAPI-backend architecture, minus the Node.js dependency.

- **JSON endpoints**: `GET /api/overview`, `/api/health`, `/api/hotspots`,
  `/api/decisions`, `/api/symbols`, `/api/wiki-pages`, `/api/wiki`,
  `/api/search`, `/api/graph`, `/api/ownership`, `/api/dead-code`, plus
  `POST /api/chat` — the same data the static dashboard's sections
  already compute (`repowise overview`/`health`/`hotspots`/`decisions`,
  plus the full symbol list), as JSON instead of baked into one static
  HTML page. File paths are always relative to `PATH`, never absolute
  host paths. `/api/hotspots` returns `{"available": false}` (not an
  error) when `PATH` isn't a git repo, same "degrade gracefully"
  behavior as the static dashboard. `/api/wiki-pages` lists which
  indexed files already have a `repowise-docs` wiki page on disk;
  `/api/wiki?path=<rel>` serves one page's raw markdown (matched
  against that exact set, so a crafted `path` can't escape
  `.repowise/wiki/` via `..` segments); `/api/search?q=<term>` does a
  case-insensitive substring match over file paths and symbol names,
  capped at 20 results each — PageRank-biased (issue #63's cheaper-than-
  embeddings intermediate step): among equally-matching results, files
  with more dependents (`repowise-graph`'s already-computed
  `dependents_of`) and symbols with more callers (`call_in_degree`)
  rank first, no new analysis or network call needed, so instant
  search stays instant; `/api/graph` returns the file-level import
  graph (nodes + edges), truncated to the 150 most-connected files
  (`"truncated": true` when cut down) so a large repo's graph stays
  renderable; `/api/ownership?path=<rel>` returns one file's git-blame
  author breakdown (`{"available": false}` for a non-git-repo or
  unindexed path, same degrade-gracefully convention); `/api/decisions`
  takes an optional `?file=<rel>` to filter to decisions linked to one
  file (omitted, it behaves exactly as before — every mined decision);
  `/api/dead-code` returns confidence-tiered dead-code candidates with
  an optional `?min_confidence=low|medium|high` filter, mirroring the
  `get_dead_code` MCP tool's own shape (`total_matching` before the
  50-candidate cap). `POST /api/chat` takes `{"history": [{"role",
  "content"}, ...]}` (the whole conversation so far) and returns
  `{"available": bool, "reply": string | null}` — `available: false`
  when `REPOWISE_LLM_BASE_URL` isn't set, the same opt-in convention
  `repowise generate` (issue #61) already uses. When available, the
  latest user message is grounded with real embeddings-based retrieval
  (issue #63): `repowise-llm::embed` calls the same endpoint's
  OpenAI-compatible `POST /v1/embeddings` to embed the question and
  every indexed file's symbol list in one batched request, ranks files
  by cosine similarity, and includes the top 10 in the system prompt.
  No vector index or persistence — every chat call re-embeds the whole
  corpus, an honest cost/latency tradeoff for a first slice. Falls back
  to the previous lightweight keyword search over file paths and symbol
  names if the embeddings call itself fails (e.g. an endpoint that
  doesn't implement `/v1/embeddings` at all), so a chat reply is never
  blocked by that. `REPOWISE_EMBEDDING_MODEL` selects the embedding
  model/route alias (default `"embed"`), separate from
  `REPOWISE_LLM_MODEL`. `GET /api/reindex` / `POST /api/reindex` (issue #65's
  live job banner) expose a background reindex job: `POST` starts one
  (`repowise_parser::build_index`, the same implementation
  `init`/`update` use, so there's exactly one indexing codepath) unless
  one's already running, and both verbs return the job's current status
  — `{"status": "idle" | "running"}` or `{"status": "completed",
  "file_count", "other_file_count", "duration_ms"}` or `{"status":
  "failed", "error"}`. A bad root surfaces as `failed`, never a 500.
  `GET /api/settings` (issue #65's Settings view, read-only) returns
  `{"root", "file_count", "other_file_count", "git_available",
  "wiki_pages_available", "llm_configured", "llm_model"}` — a status
  snapshot, not a config editor: this port has no persisted repo-level
  exclusion/generation config or global server/webhook/MCP config to
  write to yet, so there's no write endpoint here. `GET /api/usage`
  (issue #65's cost-tracking view, its fifth and last bundled feature)
  returns `{"chat_call_count", "prompt_tokens", "completion_tokens",
  "total_tokens"}`, tallied across every `/api/chat` call this server
  process has handled whose response reported OpenAI-compatible
  `usage` (`repowise_llm::complete_messages_with_usage`). In-memory for
  this process only — resets on restart, not a persisted history — and
  token counts, not a dollar figure: `repowise-llm` has no per-model
  pricing table, since an OpenAI-compatible endpoint (`rusty_provider`
  or otherwise) can route to whichever provider it's configured for.
- **Detail views** (issue #263) for symbols and decisions, deep-linkable at
  `#/symbols?id=<file>@<line>` and `#/decisions?id=<id>`, reached by clicking a
  row in either index.

  Symbol detail shows kind, parent, location, complexity and nesting, plus
  resolved callers and callees — and **states how many calls it couldn't
  resolve**, so an empty callee list is never read as "this calls nothing".
  For the same reason, "no resolved callers" says explicitly that heuristic
  resolution makes that not proof of disuse.

  Decision detail shows the full mined text, linked files, and **both
  directions of supersession lineage**. A superseded decision leads with a
  loud marker and a link to what replaced it: displaying one silently would
  read as current guidance, which is the one way this view could actively
  mislead. An unknown id renders a not-found state rather than an error.

- **One view per section, addressable** (issue #259). Every section used to
  render stacked on a single scrolling page; each now has its own route at
  `#/<view>` (`#/health`, `#/coverage`, `#/files`, …), with a nav to switch
  between them. Reload restores the current view instead of resetting to
  Overview, and the selected file rides along as `?file=<path>` so a
  drill-down survives a refresh and can be linked to.

  **Hash routing, not path routing, and deliberately.** `serve-dashboard`
  serves static files, so `/health` would 404 on reload unless the server grew
  a catch-all rewrite to `index.html`. A hash survives a reload with no server
  change at all — and the app already used a hash for present mode, so this
  follows the existing convention rather than adding a second one. It also
  needs **no new dependency**: no router crate, and a six-character
  percent-encoder for the paths that would otherwise split the hash.

  An address matching no view renders an explicit **not-found** state rather
  than silently falling back to Overview — a stale bookmark should say so.
  `#present/<n>` is left alone as an overlay, so entering and leaving present
  mode returns you to the view you were on.

- **Activity view** (issue #262) renders `GET /api/stats`: a day×hour commit
  punch card and a weekly-commit trend, both hand-drawn as inline SVG (no
  charting dependency in a WASM binary).

  **Everything is bucketed in UTC, and the view says so.** Git stores an author
  timezone offset that this port doesn't carry, so a local-time punch card
  isn't derivable — and silently bucketing in whatever timezone the server
  happens to run in would make the chart's meaning shift with the host's `TZ`.

  **A shallow clone is surfaced as a caveat in the view**, not silently
  under-reported. Truncated history doesn't make these charts fail, it makes
  them wrong in a way that looks fine — on a shallow clone of this repo, every
  commit lands in the current week, which is an artifact rather than a finding.
  Cell opacity carries magnitude while the `<title>` carries the count, so the
  value is never colour-only.

- **Search** (issue #260) is a Ctrl/Cmd+K box over `GET /api/search`, already
  PageRank-biased (#63). Requests are **debounced by 200ms**: previously every
  keystroke issued one, so typing "parser" fired six requests, five of them
  obsolete before they returned. A further keystroke re-runs the resource and
  drops the in-flight future before the delay elapses, so only a pause in
  typing actually queries. An empty box shows a prompt rather than nothing —
  an empty panel reads as "no matches" when you simply haven't typed yet — and
  symbol results are links like file results, since a result you can't act on
  is half a result.

- **Files view** (issue #261) renders `GET /api/files` as a treemap: area
  proportional to a file's line count, fill by health band. It answers what the
  ranked tables can't — where the mass of the codebase sits, and whether the
  big parts are healthy. A "10 worst files" table hides one large mediocre file
  behind ten small terrible ones.

  Layout is a hand-written [squarified
  treemap](https://www.win.tue.nl/~vanwijk/stm.pdf) (~60 lines) rather than a
  charting dependency — a WASM binary shouldn't grow one for this. Slice-and-
  dice was rejected because it degenerates into unreadable slivers well before
  this repo's 85 files. The layout is pure and deterministic, so the view
  doesn't reshuffle between loads, and it's unit-tested for area conservation
  and in-bounds tiles.

  **`unscored` is its own band**, never folded into "good" — a file with no
  health score is not a healthy file, and coloring unknown risk green would be
  worse than leaving it out. Every tile names its band in a `<title>`, so
  **color is not the only channel** carrying the information, and the legend
  names the bands rather than only showing swatches.

- **Contributors view** (issue #258) renders `GET /api/contributors`:
  per-author owned lines and share, files touched, and the repo's
  distribution of per-file bus factors. Bus factor is shown in words, not as
  a bare number — "1" reads to some as "one clear owner", the opposite of what
  it means.

  `ownership_of` shells out to `git blame` **once per file**, so the sweep is
  bounded to the 200 largest files rather than cached (a cache would need an
  invalidation story that git history doesn't have; a bound is stateless and
  its cost is knowable). The response reports **two different shortfalls
  separately** — whether the bound truncated the sweep, and how many files
  simply couldn't be blamed (untracked or never committed). They're distinct
  facts, and reporting "bounded sample" on a repo where the bound never
  applied would be wrong.

- **Coverage view** (issue #257) renders `GET /api/coverage`: mean line
  coverage, the least-covered files, and whether a per-test map is present
  (without one, `repowise impacted-tests` can't run). It states the count of
  indexed files that appear in **no** report separately from the measured
  ones — an unmeasured file is not a 0%-covered file, and the endpoint keeps
  the two apart rather than flattening them into one list. With nothing
  ingested the view says so and points at `repowise coverage add`.
- **`repowise-web`** is a companion Leptos (Rust/WASM) frontend crate that
  renders every section the static dashboard has — overview, code
  health, hotspots, architectural decisions, and a symbols table with a
  live (client-side-reactive, not just embedded-JS) kind filter — plus
  things the static dashboard didn't have a live version of yet: every
  rendered file path (including graph nodes) is a **drill-down link**
  that opens a **file-detail panel** — the file's wiki page (if
  `repowise docs` has generated one), its git-blame ownership
  breakdown, and any architectural decisions linked to it, each loading
  and failing independently so a file with no wiki page yet still shows
  whatever ownership/decision data exists instead of one shared error.
  A **Ctrl/Cmd+K instant search box** live-queries `/api/search` as you
  type and opens the same panel for a matching file. A
  **dependency-graph view** renders `/api/graph` as an SVG, laid out
  client-side with a small Fruchterman-Reingold-style force-directed
  simulation (nodes repel each other, edges act as springs, gently
  pulled back toward center) — colored by language using GitHub's own
  per-language colors, no D3 or other JS graph library involved. A
  **dead-code section** lists `/api/dead-code`'s candidates with a
  minimum-confidence filter, each risk factor available as a tooltip. A
  **chat section** talks to `POST /api/chat`, with full conversation
  history kept client-side and resent every turn; if the server reports
  the LLM isn't configured, it shows a plain explanatory message instead
  of a broken-looking chat box. A **Present Mode** (button, top of the
  page) steps full-screen through Overview/Health/Hotspots/Decisions/
  Graph with the arrow keys (`Esc` to exit) — the current slide is
  reflected in the URL as `#present/<n>`, so a link to a specific slide
  is shareable/bookmarkable. Frontend-only, no new server endpoint. A
  **live job banner** (button, top of the page, next to Present) posts
  to `/api/reindex` and polls it (via `gloo-timers`, every 500ms) until
  the background job finishes, showing "Reindexing...", a completion
  summary, or the error message on failure; it also polls once on page
  load so a job already running from a previous visit still shows up. A
  **Settings section** (bottom of the page) is a read-only status view
  over `/api/settings`: repo root, indexed file counts, and whether git
  history, wiki pages, and an LLM are available — no edit form, since
  this port has no persisted config to write to yet. A **Usage section**
  polls `/api/usage` every 3s for running chat-call and
  prompt/completion/total token counts, so it keeps reflecting the chat
  section's activity elsewhere on the page without the two components
  sharing state directly — token counts only, current server process
  only, not a persisted dollar-cost history.
  It is **the single frontend for this port** — there is deliberately no
  second one. A React/Vite app briefly existed under `web/` and was
  removed: it duplicated this crate against the same `/api/*` surface
  while being absent from CI, the README, and the server's
  `--static-dir` wiring, so it could rot or break with nothing to catch
  it. One frontend that is actually gated beats two where only one is.

  It's deliberately **not** a member of the root Cargo workspace (its
  own `Cargo.toml` has an empty `[workspace]` table): it only ever
  targets `wasm32-unknown-unknown` via [`trunk`](https://trunkrs.dev),
  and pulling a WASM-only crate into the main workspace would break
  plain `cargo build/test/clippy --workspace` for every other crate
  (which target the host).

  That exclusion has a consequence worth stating: the workspace-wide
  `Format`/`Clippy`/`Test` steps in CI **skip this crate**, so it carries
  its own `Format (WASM web crate)` and `Clippy (WASM web crate)` steps
  in `.github/workflows/ci-rust.yml`, run with `--manifest-path` against
  the wasm target. Without those it would be held to a weaker standard
  than every other crate in the repo — which is exactly the state it was
  in until now (a bare `cargo check`, no fmt, no lints).

  Build it with:
  ```sh
  rustup target add wasm32-unknown-unknown   # once
  cargo install trunk                        # once
  cd crates/repowise-web && trunk build
  ```
  then point the server at the built assets:
  ```sh
  repowise serve-dashboard [PATH] --static-dir crates/repowise-web/dist
  ```
  Omit `--static-dir` to run the JSON API alone, with no frontend served
  — useful for exercising the API directly (`curl
  http://127.0.0.1:8080/api/overview`) without building anything.
- Chosen deliberately over a real Next.js/React frontend to keep the
  whole project buildable with just `cargo` (no npm/Node dependency for
  contributors or CI) while still getting a live server, a real
  client-side app, drill-down links, instant search, and a dependency
  graph — real repowise reaches for D3.js for its graph view; this port
  gets there with a hand-rolled force-directed layout instead, so no JS
  build toolchain is needed for any part of the frontend.

This closes out issue #59 and issue #65 both: every view the static
dashboard had now has a live equivalent, plus drill-down, search, a
dependency graph, chat, Present Mode, a live job banner, a read-only
Settings view, and cost tracking — all five of #65's bundled,
live-server-dependent features the static dashboard never had. Chat's
retrieval is now real embeddings-based semantic search (issue #63's
first slice) rather than keyword matching, and `/api/search` (the
dashboard's own instant search box) is PageRank-biased by
`repowise-graph`'s already-computed in-degree data (#63's second
slice) rather than plain alphabetical — no embeddings there, since an
API call per keystroke would make instant search not instant. One
honest caveat remains: cost tracking is in-memory per-process
token counts, not a persisted dollar-cost history (no per-model
pricing table exists to convert tokens to dollars, and nothing here
survives a server restart). This still isn't a byte-for-byte
reproduction of real repowise's dashboard either (e.g. no D3-identical
graph rendering) — it's parity in what the dashboard *does*, built a
different way.

## Multi-repo workspace support

Issue #64's first slice: this port is single-repo scoped throughout its
entire architecture (`RepoIndex`, `RepoGraph`, every CLI command, and
the MCP/dashboard servers all take one root), and this doesn't change
that — it adds the smallest useful piece of the workspace concept on
top: naming a set of repo roots and reporting each one's indexed
status, via a `--workspace <path>` flag.

- **`repowise-workspace`** parses a small standalone TOML file naming
  member repos by name and path:
  ```toml
  [[repo]]
  name = "rusty_repo_wise"
  path = "/home/user/rusty_repo_wise"

  [[repo]]
  name = "some_other_repo"
  path = "../some_other_repo"
  ```
  `path` may be relative to the workspace file's own directory (not the
  process's current directory). This file is never inferred from or
  stored inside any member repo's own `.repowise/` — a workspace spans
  repos, so no single member repo is a sensible owner of it.
- `repowise workspace-repos --workspace <path>` lists every configured
  repo with its indexed status and file count if a prior `repowise
  init`/`update` has run there.
- `repowise serve --workspace <path>` (the MCP server) gains a
  `list_repos` tool reporting the same thing as agent-facing JSON.
  Omitting `--workspace` means `list_repos` returns an empty list
  rather than erroring.
- `repowise serve-dashboard --workspace <path>` gains `GET
  /api/workspace-repos` (`{"available": bool, "repos": [...]}` — same
  degrade-gracefully shape as `/api/hotspots`/`/api/chat`) and the
  dashboard gets a **Workspace section** (repo cards: name, path,
  indexed status, file counts).

The next slice adds workspace co-change reporting: each configured
repo's own most-coupled file pairs (via `repowise-git`'s existing
`GitAnalytics`), shown side by side. This is deliberately **not**
cross-repo co-change — separate repos have separate git histories, so
files in different repos can never literally co-change in the same
commit — just each repo's own coupling rendered together in one place,
no new dependency resolution required.

- `repowise workspace-co-changes --workspace <path>` (`--top <N>`,
  default 10) prints each repo's most-coupled file pairs, or a note when
  a repo has no readable git history.
- `repowise serve-dashboard --workspace <path>` gains `GET
  /api/workspace-co-changes` (same `{"available", "repos": [{"name",
  "path", "available", "pairs": [{"file_a", "file_b", "count"}]}]}`
  shape) and the dashboard gets a **Workspace Co-Changes section**.

The next slice adds the real thing: cross-repo Rust `use` resolution.
`repowise-graph` already builds a per-crate Rust module-path map
(`crate::path` -> file) from each repo's `Cargo.toml`; merging those
maps across workspace repos lets an unresolved `use other_crate::Thing`
in one repo resolve to a real file in another. Rust-only for now — the
only language this port anchors to a `Cargo.toml`-derived crate name;
every other language's cross-repo imports are left unresolved,
deliberately, for a future slice.

- `repowise workspace-architecture --workspace <path>` prints each
  repo's indexed status, a repo-pair dependency summary, and the
  individual cross-repo import sites behind each dependency.
- `repowise workspace-blast-radius --workspace <path> --repo <name>
  --file <path>` prints the other repos' files that directly (one hop,
  not transitive — matching `RepoGraph::dependents_of`'s existing
  single-repo precedent) cross-repo-import the given file.
- `repowise serve --workspace <path>` (the MCP server) gains
  `get_architecture` (degrades to empty lists like `list_repos`) and
  `get_blast_radius` (errors like `get_context` on an unknown repo or
  unindexed file — it targets one specific file, not a repo-wide view).
- `repowise serve-dashboard --workspace <path>` gains `GET
  /api/workspace-architecture` and the dashboard gets a **System Map
  section** (a repo-pair table with the individual import sites listed
  underneath — a plain table, not a force-directed graph, since
  repo-level granularity is small).

The next slice adds conformance: circular cross-repo dependencies,
reusing exactly the edges `workspace-architecture` already computes. A
workspace's repo-level dependency graph should form a DAG; a cycle
(repo A imports repo B imports repo A, or a longer chain) is a
concrete, deterministic "pattern divergence" finding that needs no
further human-specified rule set to detect.

- `repowise workspace-conformance --workspace <path>` prints any
  circular cross-repo dependencies found, or a "none found" message.
- `repowise serve-dashboard --workspace <path>` gains `GET
  /api/workspace-conformance` and the dashboard gets a **Conformance
  section**.

The final slice adds contracts: producer/consumer API contract
matching. Unlike every other #64 slice, this needs no cross-repo
symbol resolution at all — it's a regex-based scan of each indexed
file's raw text for a small, fixed table of HTTP route-registration
patterns (axum `.route("/path", get(...))`, Flask/FastAPI
`@app.get("/path")`, Express `app.get("/path", ...)`) and HTTP-call
patterns (JS `fetch("/path")`/`axios.get("/path")`, Python
`requests.get("/path")`, Rust `ureq::get("/path")`), matching each
consumer call against producer routes registered in *other* repos
(segment-wise, treating a producer path segment like `:id`/`{id}` as a
wildcard). Coarse and heuristic by design — a real implementation would
need to parse each web framework's actual route-registration semantics
per language, which this port has no such capability for. False
negatives (an unrecognized framework idiom) and false positives (a
route-shaped string that isn't actually a route) are both expected.

- `repowise workspace-contracts --workspace <path>` prints matched
  producer/consumer pairs and any consumer calls with no known producer
  in the workspace (not necessarily a problem — may be a genuinely
  external API, or a producer this heuristic doesn't recognize).
- `repowise serve-dashboard --workspace <path>` gains `GET
  /api/workspace-contracts` and the dashboard gets a **Contracts
  section**.
- This closes out all five items #64 originally bundled. There's still
  no way to switch which repo the rest of the dashboard/MCP server
  operates on — `root` stays fixed for the life of the process.

## Testing

```sh
cargo test --workspace
```

Includes parser unit tests (function/class/import/call/complexity/
param-count/duplicate-hash extraction on inline source snippets), graph
integration tests that write real fixture files to a temp directory to
exercise Rust's `mod`/crate-root resolution and Python's package-relative
import resolution end to end, health-scoring tests that build
`RepoIndex` fixtures directly to exercise each marker (and the resulting
score) in isolation, git-analytics tests that build real disposable git
repos (via the `git` CLI) to exercise churn/bug-fix/co-change/ownership/
hotspot computation against actual `git log`/`git blame` output rather
than a mock of it, docs-generation tests covering page rendering content
and the New/Changed/Unchanged freshness transitions on a real temp
directory, ADR-mining tests (ADR parsing, the unfilled-template skip,
decision-commit detection, file/symbol linking, and an end-to-end test on
a real git repo covering supersession and linking together), and MCP
server tests that call each tool method directly against a real index
built by the actual indexing pipeline (not hand-built fixtures), covering
the happy path for all three tools plus the invalid-query and
unindexed-file error cases, and dashboard tests covering HTML escaping,
relative-path rendering, the graceful-degradation placeholders, and an
end-to-end render against a real indexed temp directory.
