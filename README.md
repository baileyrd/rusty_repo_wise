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
  Swift, PHP, Dart, and shell (`.sh`/`.bash`/`.zsh`) files.
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
- Score every file's health deterministically (0–10, no LLM/ML) from six
  rule-based markers: long functions, high cyclomatic complexity, oversized
  parameter lists, god classes, duplicate code, and possibly-dead code
  (zero resolved callers) — except for shell scripts, which are
  deliberately exempt from the dead-code marker: a shell function is
  routinely invoked only from the command line, another script, or a
  cron job, none of which this port's call graph can see, making the
  signal too unreliable to report for that language.
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
Scala, Ruby, Swift, PHP, Dart, and shell scripts are parsed; repowise's
other languages aren't implemented — see issue #11 for the
tracking/discussion issue on extending language support. The health scorer covers 6 of repowise's ~25 markers — see
"Health scoring" below for which ones and why the rest (LCOM4 cohesion,
Rabin-Karp substring clone detection) are deferred. LLM-written prose on
top of the wiki (`repowise generate` in the original) is also deferred —
this port's `docs` layer is deliberately deterministic-only, as is ADR
mining (only 4 of the original's 8 decision sources are implemented —
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
- `repowise-adr` — architectural-decision mining from ADR files,
  decision-like commit messages, decision-like merged PR bodies (via the
  GitHub API, opt-in behind a token env var), and decision-like code
  comments, linked to the files/symbols they mention.
- `repowise-mcp` — an MCP server (via the official `rmcp` SDK) exposing
  the index/graph/health/git-analytics/mined-decisions data, plus a
  deterministic per-commit change-risk score and confidence-tiered
  dead-code candidates, as agent-facing tools over stdio.
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
repowise serve [PATH]               # run an MCP server over stdio (get_overview/search_codebase/get_context/get_risk/get_change_risk/get_symbol/get_why/get_dead_code)
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

"Possibly dead code" is never applied to shell scripts (`Language::Shell`)
— a shell function is routinely invoked only from the command line,
another script, or a cron job, none of which this port's call graph can
see, so the signal is too unreliable to report for that language. All
other markers still apply to shell the same as everywhere else.

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

`repowise decisions` mines four of the original's eight decision sources:

- **`docs/adr/*.md` files**, parsed against this repo's own ADR template
  (`# ADR-XXXX: Title`, then `Status:`/`Date:` lines). An unfilled
  template (title still literally `<Title>`) is skipped rather than mined
  as a real decision.
- **Decision-like commit messages** — a message containing one of a small
  keyword set (`decide`, `decision`, `chose`, `chosen`, `switch to`,
  `adopt`, `instead of`). A heuristic, not ground truth, same framing as
  the bug-fix-commit detection in git analytics.
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

Each ADR-file/commit-message decision is linked to the indexed files it
mentions: either the file's own relative path appearing verbatim in the
decision's body text, or one of its non-module symbol names (4+
characters, to cut down on false positives from short identifiers)
appearing as a whole word. Matching text, not meaning — a decision that
only refers to a file descriptively ("the queue module") won't be
linked. Supersession is read directly from an ADR's `Status: Superseded
by ADR-XXXX` line — no new front-matter convention was needed since the
existing template already has one.

Not implemented from the original's eight sources: inline decision
markers (`# WHY:`, `# DECISION:`, etc.), CHANGELOG mining, Slack, and
issue trackers — this repo doesn't have integrations for the
latter two anyway. Recency/confidence scoring on mined decisions is also
not implemented.

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
  bodies, and decision-like code comments (via `repowise-adr`), the same
  data as `repowise decisions --for-file`. `targets` is a list
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
