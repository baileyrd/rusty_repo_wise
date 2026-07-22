# rusty_repo_wise

A Rust-native reimplementation inspired by [repowise](https://github.com/repowise-dev/repowise),
a codebase-intelligence platform that builds dependency graphs, git
analytics, auto-generated docs, architectural-decision tracking, and
deterministic code-health scoring for AI agents and developers.

This is a fresh, from-scratch Rust project — it does not share code or a
license with the original (AGPL-3.0) repowise.

## Scope so far

repowise is a large product with five "intelligence layers," an MCP
server, and a web dashboard. So far this port builds the **core CLI, the
dependency-graph layer, and the code-health-scoring layer**, end to end:

- Walk a codebase (respecting `.gitignore`), detect Rust and Python files.
- Parse each file with tree-sitter, extracting function/method/class/struct
  definitions, imports, call expressions, and per-function metrics
  (cyclomatic complexity, parameter count, a duplicate-code body hash).
- Resolve imports and calls into a dependency graph (files and symbols as
  nodes; `Contains`/`Imports`/`Calls` edges), using directory-layout
  conventions (Rust's `mod`/crate-root rules, Python's package layout) —
  **not** full compiler-grade name resolution. Ambiguous or external
  references are left unresolved rather than guessed.
- Score every file's health deterministically (0–10, no LLM/ML) from six
  rule-based markers: long functions, high cyclomatic complexity, oversized
  parameter lists, god classes, duplicate code, and possibly-dead code
  (zero resolved callers).
- Persist the index to `.repowise/index.json` and query it from the CLI.

Not yet built: git analytics/hotspots, doc generation, ADR mining, the
MCP server, and the web dashboard. Only Rust and Python are parsed;
repowise's other 14 languages aren't implemented. The health scorer
covers 6 of repowise's ~25 markers — see "Health scoring" below for which
ones and why the rest (mostly git-history-based, like churn/hotspots, and
LCOM4 cohesion) are deferred.

## Crates

- `repowise-core` — shared data model (`Symbol`, `FileRecord`, `RepoIndex`,
  etc.), `.gitignore`-aware file discovery, and JSON index persistence.
- `repowise-parser` — tree-sitter-based extraction for Rust and Python,
  including per-function complexity/param-count/body-hash metrics.
- `repowise-graph` — builds the dependency graph from a `RepoIndex` and
  answers overview/search/deps/call-in-degree queries.
- `repowise-health` — deterministic code-health scoring built on top of
  the parsed metrics and the call graph.
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
| Possibly dead code | 0 resolved callers | −0.2 |

All of these come from data already computed by `repowise-parser`
(per-symbol line span, complexity, param count, body hash) and
`repowise-graph` (call-graph in-degree) — no new heuristics are hidden
inside the scorer itself. "Possibly dead code" and "duplicate code" are
intentionally low-weighted since they inherit the graph layer's
best-effort call/import resolution: a symbol can look uncalled just
because a call site couldn't be resolved (trait dispatch, dynamic
dispatch, an external caller), not because it's truly unused.

Deferred markers from the original repowise (not implemented): churn ×
complexity hotspots, ownership/co-change coupling, and bug-fix history
(all need git-log analysis — a separate "git analytics" layer); LCOM4
cohesion (needs field-level access tracking per method); Rabin-Karp
substring clone detection (this port only detects whole-body duplicates
via exact hash match, not partial/near-duplicate code).

## Testing

```sh
cargo test --workspace
```

Includes parser unit tests (function/class/import/call/complexity/
param-count/duplicate-hash extraction on inline source snippets), graph
integration tests that write real fixture files to a temp directory to
exercise Rust's `mod`/crate-root resolution and Python's package-relative
import resolution end to end, and health-scoring tests that build
`RepoIndex` fixtures directly to exercise each marker (and the resulting
score) in isolation.
