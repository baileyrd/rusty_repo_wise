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
| Per-language extraction (`repowise_parser::parse_file`) | `rust::extract`, `python::extract`, `javascript::extract_typescript`, `javascript::extract_javascript`, `java::extract`, `kotlin::extract`, `go::extract` | callers match on `Language` and never touch per-language internals directly; adding a language means adding a match arm, not implementing a trait |
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
the raw index and commit history. `repowise-mcp` depends on
`repowise-core`/`repowise-graph`/`repowise-health` (it's a thin transport
layer wrapping their existing query functions as MCP tools) plus `rmcp`
(the official Rust MCP SDK) and `tokio` — the only crates in this
workspace with an async runtime dependency; `repowise-cli` builds a
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
`.repowise/index.json`. Every other command (`overview`, `search`, `deps`,
`health`, `docs`) loads that index, builds a `RepoGraph` (resolves
imports/calls into `Contains`/`Imports`/`Calls` edges), and queries it —
`repowise-health` adds one more pass over the graph's symbols and
call-in-degrees to score files, and `repowise-docs` renders one markdown
page per file from the index/graph/health data, tracking freshness via a
hash of each file's own source re-read from disk (not the index) at
generation time.
`hotspots`/`ownership`/`coupled` are a separate path: they load the same
`RepoIndex` for complexity data, but get their git-history data by shelling
out to `git log`/`git blame` fresh on every invocation rather than reading
anything cached in `.repowise/index.json` — see ARCHITECTURE's "Non-goals"
and the README's "Git analytics" section for why (staleness/invalidation
complexity not worth taking on yet).
`decisions` is a third, independent path: `repowise-adr` reads `docs/adr/*.md`
directly off disk and reuses `repowise-git::collect_commits` for raw commit
messages (not `RepoIndex`, which only has complexity metrics, not commit
history), then links each decision's body text to files/symbols in the
same `RepoIndex` the other commands use.
`serve` is a thin wrapper over the same `overview`/`search`/`deps`/`health`
data paths, re-exposed as MCP tools: `repowise-mcp` loads `RepoIndex`,
builds a `RepoGraph`, and (for `get_context`) runs `repowise_health::analyze`
fresh on every tool call — no state held across calls, no caching.
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
