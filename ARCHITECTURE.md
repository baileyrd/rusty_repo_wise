# Architecture

## Overview
A Rust CLI (`repowise`) that indexes a codebase and answers questions about
it: dependency graph queries, deterministic code-health scores, git-history
analytics (churn/hotspots/ownership/coupling), auto-generated per-file
documentation, and architectural-decision mining — plus an MCP server
exposing a subset of that as agent-facing tools over stdio, and a
static-site dashboard summarizing it all in one HTML page. See the root
README for current scope and what's deliberately deferred per layer.

## Boundaries
This is a straightforward layered pipeline, not a plugin-style
ports-and-adapters system with `dyn Trait` boundaries — worth saying plainly
rather than forcing that framing where it doesn't fit. The real seams are
match-based dispatch points, each with exactly one implementation per branch
today:

| Seam | Implementation(s) | Notes |
| ---- | ------------------ | ----- |
| Per-language extraction (`repowise_parser::parse_file`) | `rust::extract`, `python::extract`, `javascript::extract_typescript`, `javascript::extract_javascript`, `java::extract`, `kotlin::extract`, `go::extract`, `cpp::extract`, `csharp::extract`, `scala::extract`, `ruby::extract`, `c::extract`, `swift::extract`, `php::extract`, `dart::extract`, `shell::extract` | callers match on `Language` and never touch per-language internals directly; adding a language means adding a match arm, not implementing a trait |
| File discovery (`repowise_core::discover_files`) | wraps the `ignore` crate | `.gitignore`-aware; the walker itself isn't swappable yet |
| Index persistence (`RepoIndex::save`/`load`) | JSON on disk (`.repowise/index.json`) | the one and only backing store so far |

## Structure
Modular monolith: one Cargo workspace, ten crates. Most are a layer over
the one below it — `repowise-core` (data model, file discovery, index
persistence) → `repowise-parser` (tree-sitter extraction) → `repowise-graph`
(dependency graph + queries) → `repowise-health` (deterministic scoring on
top of the graph) → `repowise-docs` (renders `repowise-health`'s findings
and `repowise-graph`'s deps/dependents into per-file pages) — all tied
together by `repowise-cli` (the binary). Three crates sit outside the strict
pipeline: `repowise-git` depends only on `repowise-core` (for `RepoIndex`'s
per-symbol complexity) and the `git` CLI directly, not on `repowise-graph`,
since git-history analysis doesn't need the dependency graph at all.
`repowise-adr` depends on `repowise-core` (for symbol/file linking) and
`repowise-git` (to reuse its commit-log parsing for decision-mining rather
than duplicating it), but not `repowise-graph`/`repowise-health` — decision
mining doesn't need the resolved dependency graph or health scores, just
the raw index and commit history. It also depends on `ureq` (a
synchronous HTTP client, chosen over an async one like `reqwest`
specifically to avoid pulling `tokio` into a crate that's otherwise
plain git/filesystem work) for its one network-dependent source: mining
merged PR bodies via the GitHub API, gated behind an explicit
`REPOWISE_GITHUB_TOKEN` env var (see the README's "Architectural
decision mining" section for why that's opt-in rather than falling back
to an unauthenticated API call). `repowise-mcp` depends on
`repowise-core`/`repowise-graph`/`repowise-health`/`repowise-git`/
`repowise-adr` (it's a thin transport layer wrapping their existing query
functions as MCP tools — `get_risk` and `get_change_risk` are the two
tools that need `repowise-git`'s data. `get_risk` degrades to zero/empty
hotspot/churn/bug-fix data rather than erroring when the indexed root
isn't a git repository; `get_change_risk` is pure diff-shape analysis
with no index dependency at all — it errors instead, since there's no
diff to compute without a git repository. `get_why` is the one tool that
needs `repowise-adr`'s mined-decision data, calling `repowise_adr::mine`
fresh on every call just like every other tool re-loads the index fresh)
plus `rmcp` (the official
Rust MCP SDK) and `tokio` — the only crates in this workspace with an
async runtime dependency; `repowise-cli` builds a
`tokio::runtime::Runtime` manually just for the `serve` subcommand rather
than making the whole synchronous CLI async. `repowise-dashboard` sits at
the top of the pipeline, depending on `repowise-core`/`repowise-graph`/
`repowise-health`/`repowise-git`/`repowise-adr` — it's purely a rendering
layer over everything else's already-computed output, with no logic of
its own beyond HTML templating (plain `format!`, no templating-engine
dependency) and HTML-escaping untrusted text. No crate has been split out
as a separate service or process (`repowise-mcp`/`repowise-dashboard` are
libraries invoked by the CLI, not standalone binaries/servers); there's no
forcing function (scaling, team boundary, fault isolation) that would
justify it yet.

## Data flow
`init`/`update` → `discover_files` walks the tree → `repowise_parser::parse_file`
extracts symbols/imports/calls per file into a `RepoIndex` → saved to
`.repowise/index.json`. For Rust/Python/TypeScript/JavaScript specifically,
extraction also walks each method body for `self`/`this` field
reads/writes into `FileRecord::field_accesses` (empty for the other 12
parsed languages, which don't extract it yet). Every other command
(`overview`, `search`, `deps`, `health`, `docs`) loads that index, builds a
`RepoGraph` (resolves imports/calls into `Contains`/`Imports`/`Calls`
edges), and queries it — `repowise-health` adds one more pass over the
graph's symbols and call-in-degrees to score files (plus a separate pass
over `field_accesses` alone for the LCOM4 low-cohesion marker, which
doesn't need the call graph at all — see `repowise-health::lcom4`), and
`repowise-docs` renders one markdown page per file from the index/graph/
health data, tracking freshness via a hash of each file's own source
re-read from disk (not the index) at generation time.
`hotspots`/`ownership`/`coupled` are a separate path: they load the same
`RepoIndex` for complexity data, but get their git-history data by shelling
out to `git log`/`git blame` fresh on every invocation rather than reading
anything cached in `.repowise/index.json` — see ARCHITECTURE's "Non-goals"
and the README's "Git analytics" section for why (staleness/invalidation
complexity not worth taking on yet).
`decisions` is a third, independent path: `repowise-adr` reads `docs/adr/*.md`
directly off disk, reuses `repowise-git::collect_commits` for raw commit
messages (not `RepoIndex`, which only has complexity metrics, not commit
history) — only when a `REPOWISE_GITHUB_TOKEN` env var is set — calls
the GitHub API for merged PR bodies (the one network call anywhere in
this port's non-MCP command paths), and re-reads each indexed file's
source fresh from disk twice more: once to mine decision-like comments
sitting directly above a symbol (`code_comments.rs`), and once to scan
every comment line in the file for an inline decision marker
(`inline_markers.rs`) — comment text isn't kept in `RepoIndex` any more
than a symbol's own source is, the same tradeoff `get_symbol`/
`repowise-docs` already make. The two comment-mining modules solve
different problems on purpose rather than sharing one scanner:
`code_comments::comment_block_above` answers "what's the comment block
directly above this specific symbol", while `inline_markers::comment_lines`
answers "every comment line in the file, wherever it sits" — forcing the
inline-marker source through the former would mean calling it once per
symbol and still needing a separate whole-file scan for markers that
aren't adjacent to a declaration at all. `repowise-adr` also reads
whichever `CHANGELOG.md`/`HISTORY.md`/`NEWS.md`/`CHANGES.md` it finds
first at the repo root (`changelog.rs`) for keep-a-changelog-style
`### Changed`/`### Removed`/`### Deprecated`/`### Security` sections.
ADR-file, commit-message, and changelog decisions get linked to
files/symbols in the same `RepoIndex` the other commands use (a
changelog entry isn't "about" the changelog file itself the way a PR's
diff or a comment's enclosing file is, so it gets this text-matched
treatment rather than an authoritative self-link); PR, code-comment, and
inline-marker decisions skip that step, already linked to the files the
GitHub API reports that PR touched, or the file the comment/marker sits
in, respectively.
`serve` is a thin wrapper over the same `overview`/`search`/`deps`/`health`
data paths, re-exposed as MCP tools: `repowise-mcp` loads `RepoIndex`,
builds a `RepoGraph`, and (for `get_context`/`get_risk`) runs
`repowise_health::analyze` fresh on every tool call — no state held
across calls, no caching. `get_change_risk` is the one tool that bypasses
this path entirely: it never loads the index or graph, calling straight
into `repowise-git`'s diff-shape analysis (`git diff`/`git show`/
`git rev-list`) against the indexed root. `get_symbol` loads `RepoIndex`
(to look up the requested symbol's file and line span) but re-reads that
file's source fresh from disk to slice out the snippet — the same
"don't trust the index's own copy of file content, only its line
metadata" tradeoff `repowise-docs`'s freshness tracking already makes.
`get_why` calls `repowise_adr::mine` fresh on every call, the same
independent path `decisions` already uses, then filters the result by
whether each decision's `linked_files` intersects the requested targets'
resolved files — no new mining logic, purely a filter over existing
output.
`get_dead_code` calls a new `repowise_health::find_dead_code`, a richer
sibling to `analyze`'s `possibly-dead-code` marker: both start from the
same `graph.call_in_degree(...) == 0` signal, but `find_dead_code` tiers
each candidate by two extra risk factors — an ambiguous same-named
symbol elsewhere in the index, and an unresolved import whose last path
segment matches the candidate's file stem. The second factor is why
`RepoGraph` now tracks `unresolved_import_stems` (populated during
`build()` alongside the existing `unresolved_imports` counter) — the one
piece of raw resolution data neither `RepoIndex` nor the existing
`Overview` aggregate exposed.
`dashboard` composes all of the above into one static page: `repowise-dashboard`
calls the same `overview`/`analyze` functions, tries `repowise-git::GitAnalytics::collect`
and `repowise-adr::mine` (both degrading to `None`/empty on failure rather
than erroring the whole command — neither git history nor ADRs are
required for a dashboard to be useful), and renders everything into one
HTML file written once per invocation. No live queries, no incremental
regeneration.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs.

## Non-goals
- Compiler-grade name resolution — import/call resolution is directory-layout
  heuristics; ambiguous or external references are left unresolved rather
  than guessed (see README).
- Feature parity with the original repowise project across any layer —
  every layer in this port (including the dashboard) covers a subset of
  the original's scope within it, not the full original (see the README
  for specifics per layer). A live/queryable dashboard (vs. this port's
  static-snapshot one) is explicitly deferred, not silently dropped — see
  the README's "Dashboard" section for what that would need.
