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
  TypeScript, JavaScript, Java, Kotlin, and Go files.
- Parse each file with tree-sitter, extracting function/method/class/struct
  definitions, imports, call expressions, and per-function metrics
  (cyclomatic complexity, parameter count, a duplicate-code body hash).
- Resolve imports and calls into a dependency graph (files and symbols as
  nodes; `Contains`/`Imports`/`Calls` edges), using directory-layout
  conventions (Rust's `mod`/crate-root rules, Python's package layout,
  TypeScript/JavaScript's relative `./`/`../` specifiers, Java/Kotlin's
  shared Maven/Gradle `src/main/java`/`src/main/kotlin`-anchored package
  paths, Go's `go.mod`-anchored module paths) — **not** full
  compiler-grade name resolution. Ambiguous or external references (npm
  packages/JVM classpath dependencies/Go modules outside the local
  `go.mod`, since there's no `node_modules`/classpath/Go-proxy
  resolution) are left unresolved rather than guessed.
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
- Mine architectural decisions from `docs/adr/*.md` files and decision-like
  commit messages, link each to the files/symbols it mentions, and track
  supersession via an ADR's `Status: Superseded by ADR-XXXX` line.
- Expose `get_overview`/`search_codebase`/`get_context` as MCP tools over
  stdio (the official `rmcp` SDK), so an agent can pull complete context
  for a file in one round-trip instead of piecing it together itself.
- Generate a static-site dashboard (one self-contained HTML page, no
  server, no JS build step) covering overview stats, code health,
  hotspots, and mined decisions — regenerate by re-running the command.
- Persist the index to `.repowise/index.json` and query it from the CLI.

Only Rust, Python, TypeScript, JavaScript, Java, Kotlin, and Go are
parsed; repowise's other languages (C++, C#, Scala, Ruby, and more)
aren't implemented — see issue #11 for the tracking/discussion issue on
extending
language support. The health scorer covers 6 of repowise's ~25 markers — see
"Health scoring" below for which ones and why the rest (LCOM4 cohesion,
Rabin-Karp substring clone detection) are deferred. LLM-written prose on
top of the wiki (`repowise generate` in the original) is also deferred —
this port's `docs` layer is deliberately deterministic-only, as is ADR
mining (only 2 of the original's 8 decision sources are implemented —
see "Architectural decision mining" below). The MCP server covers 3 of
the original's ~10 tools — see "MCP server" below for which and why. The
dashboard is one static page with no per-file drill-down or live search
— see "Dashboard" below for what a richer version would need.

## Crates

- `repowise-core` — shared data model (`Symbol`, `FileRecord`, `RepoIndex`,
  etc.), `.gitignore`-aware file discovery, and JSON index persistence.
- `repowise-parser` — tree-sitter-based extraction for Rust, Python,
  TypeScript, JavaScript, Java, Kotlin, and Go, including per-function
  complexity/param-count/body-hash metrics.
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
- `repowise-adr` — architectural-decision mining from ADR files and
  decision-like commit messages, linked to the files/symbols they mention.
- `repowise-mcp` — an MCP server (via the official `rmcp` SDK) exposing
  the index/graph/health data as agent-facing tools over stdio.
- `repowise-dashboard` — a static-site dashboard rendered from the
  overview/health/hotspot/decision data the other layers compute.
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
repowise decisions [PATH]          # mined ADRs + decision-like commits, with linked files
                                    #   --for-file <FILE> to filter to one file
repowise serve [PATH]               # run an MCP server over stdio (get_overview/search_codebase/get_context)
repowise dashboard [PATH]           # generate a static HTML dashboard under .repowise/dashboard
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
  symbols (complexity already computed by `repowise-parser`). `hotspots()`
  ranks files by a **recency-weighted** variant of this score: each commit
  contributes `exp(-age_days / 90)` instead of a flat `1`, so a file with
  the same raw churn as another but touched more recently ranks higher.
  The raw (non-decayed) score is still reported alongside it for
  transparency.
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

Not implemented from the original's git-analytics scope: a bug-fix
heuristic based on linked-issue references rather than just message
keywords.

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

## Architectural decision mining

`repowise decisions` mines two of the original's eight decision sources:

- **`docs/adr/*.md` files**, parsed against this repo's own ADR template
  (`# ADR-XXXX: Title`, then `Status:`/`Date:` lines). An unfilled
  template (title still literally `<Title>`) is skipped rather than mined
  as a real decision.
- **Decision-like commit messages** — a message containing one of a small
  keyword set (`decide`, `decision`, `chose`, `chosen`, `switch to`,
  `adopt`, `instead of`). A heuristic, not ground truth, same framing as
  the bug-fix-commit detection in git analytics.

Each decision is linked to the indexed files it mentions: either the
file's own relative path appearing verbatim in the decision's body text,
or one of its non-module symbol names (4+ characters, to cut down on
false positives from short identifiers) appearing as a whole word.
Matching text, not meaning — a decision that only refers to a file
descriptively ("the queue module") won't be linked. Supersession is read
directly from an ADR's `Status: Superseded by ADR-XXXX` line — no new
front-matter convention was needed since the existing template already
has one.

Not implemented from the original's eight sources: PR descriptions, code
comments, Slack, issue trackers, and three others this repo doesn't have
integrations for anyway. Recency/confidence scoring on mined decisions is
also not implemented.

## MCP server

`repowise serve [PATH]` runs an MCP server over stdio (via the official
[`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) SDK), requiring
a prior `repowise init`/`update`. Three tools are implemented:

- **`get_overview`** — the same data as `repowise overview`: file/language/
  symbol counts, edge counts, most-depended-on files.
- **`search_codebase`** — the same substring search as `repowise search`.
- **`get_context`** — a file's symbols, resolved dependencies/dependents,
  and health score/findings in one call. This is the tool that matters
  most for the original's stated goal (cutting an agent's token spend on
  context-loading): one round-trip instead of separate search/deps/health
  reads pieced together by the caller.

Every call re-loads `.repowise/index.json` and rebuilds the dependency
graph fresh — no caching across calls, consistent with how `hotspots`/
`ownership`/`coupled`/`decisions` already work in this port.

Not implemented from the original's ~10 tools: `get_risk`/`get_change_risk`
(these would read naturally on `repowise-git`'s hotspot data, now that
git analytics exists — a well-scoped next addition, deliberately left out
of this first pass rather than bundled in) and the rest of the original's
tool surface beyond what this port's other layers currently support.

## Dashboard

`repowise dashboard [PATH]` writes one self-contained static HTML page to
`.repowise/dashboard/index.html` — open it directly in a browser, no
server to run. Kept deliberately simple: a single page combining four
sections, each degrading gracefully to an explicit "not available"
placeholder (never a silently blank section) when its data doesn't exist:

- **Overview** — same data as `repowise overview`.
- **Code health** — average score, markers by kind, lowest-scoring files
  (same data as `repowise health`).
- **Hotspots** — top files by churn × complexity (same data as `repowise
  hotspots`), or a placeholder if `PATH` isn't a git repo.
- **Architectural decisions** — mined ADRs/decision-commits (same data as
  `repowise decisions`), or a placeholder if none are found.

Regenerating means re-running the command — there's no live server, no
auto-refresh, and no per-file drill-down (e.g. clicking a file to see its
`repowise-docs` wiki page). A richer version would need at minimum: a
small local HTTP server (the `tokio` dependency already exists from the
MCP server) for live queries instead of a static snapshot, and linking
each file mentioned to its rendered wiki page. Deliberately left out of
this first pass to keep the dashboard to what "generate a static site
from data we already compute" actually requires.

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
