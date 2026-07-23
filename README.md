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
dependency-graph layer, the code-health-scoring layer, the git-analytics
layer, and the auto-generated-documentation layer**, end to end:

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
- Derive git-history analytics — churn, hotspot score (churn × complexity),
  bug-fix commit frequency, co-change coupling, and per-author line
  ownership — by shelling out to `git log`/`git blame`, joined against the
  index for complexity data.
- Generate a deterministic, template-based markdown "wiki" page per file
  under `.repowise/wiki/` — symbol list, resolved dependencies/dependents,
  and health findings — with per-file freshness tracking (no LLM prose).
- Persist the index to `.repowise/index.json` and query it from the CLI.

Not yet built: ADR mining, the MCP server, and the web dashboard. Only
Rust and Python are parsed; repowise's other 14 languages aren't
implemented. The health scorer covers 6 of repowise's ~25 markers — see
"Health scoring" below for which ones and why the rest (LCOM4 cohesion,
Rabin-Karp substring clone detection) are deferred. LLM-written prose on
top of the wiki (`repowise generate` in the original) is also deferred —
this port's `docs` layer is deliberately deterministic-only.

## Crates

- `repowise-core` — shared data model (`Symbol`, `FileRecord`, `RepoIndex`,
  etc.), `.gitignore`-aware file discovery, and JSON index persistence.
- `repowise-parser` — tree-sitter-based extraction for Rust and Python,
  including per-function complexity/param-count/body-hash metrics.
- `repowise-graph` — builds the dependency graph from a `RepoIndex` and
  answers overview/search/deps/call-in-degree queries.
- `repowise-health` — deterministic code-health scoring built on top of
  the parsed metrics and the call graph.
- `repowise-git` — git-history analytics (churn, hotspots, bug-fix
  frequency, co-change coupling, ownership), computed fresh from `git
  log`/`git blame` each time it's queried rather than cached in the index.
- `repowise-docs` — deterministic per-file markdown documentation pages
  rendered from the index/graph/health data, with content-hash-based
  freshness tracking.
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
repowise hotspots [PATH]           # files ranked by churn × complexity
repowise ownership <FILE> [PATH]   # per-author line ownership (git blame)
repowise coupled <FILE> [PATH]     # files that most often change alongside it
repowise docs [PATH]               # generate per-file wiki pages under .repowise/wiki
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

Deferred markers from the original repowise (not implemented): LCOM4
cohesion (needs field-level access tracking per method); Rabin-Karp
substring clone detection (this port only detects whole-body duplicates
via exact hash match, not partial/near-duplicate code). Hotspots and
bug-fix history are now implemented (see "Git analytics" below) but
aren't yet folded into the health score itself — that's a natural
follow-up, not done here.

## Git analytics

`repowise hotspots`/`ownership`/`coupled` shell out to `git log`/`git
blame` under the hood — no persistence, no caching, just re-run each
time against the repo's real history:

- **Churn**: number of commits touching a file, from a single `git log
  --name-only` walk of the whole history.
- **Hotspot score**: `churn × total cyclomatic complexity` of the file's
  symbols (complexity already computed by `repowise-parser`). Simple and
  legible by design — no recency weighting or decay.
- **Bug-fix commits**: commits whose message contains a fix-like keyword
  (`fix`, `bug`, `hotfix`, `patch`) touching the file. A heuristic, not
  ground truth — a fix described without one of these words won't be
  counted, and an unrelated commit that happens to mention one will be.
- **Co-change coupling**: files that appear together in the same commit,
  counted per pair. Commits touching more than 50 files are skipped when
  building this (a rename sweep or vendor bump would otherwise flood
  every touched file's coupling list with noise).
- **Ownership**: per-author share of a file's lines from `git blame
  --line-porcelain`.

Not implemented from the original's git-analytics scope: recency-weighted
or decayed hotspot scoring, and a bug-fix heuristic based on linked-issue
references rather than just message keywords.

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
changed. Not implemented from the original: LLM-written prose on top of
these pages (`repowise generate`), and the dashboard's doc browser.

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
than a mock of it, and docs-generation tests covering page rendering
content and the New/Changed/Unchanged freshness transitions on a real
temp directory.
