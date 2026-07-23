# Architecture

## Overview
A Rust CLI (`repowise`) that indexes a codebase and answers questions about
it: dependency graph queries, deterministic code-health scores, git-history
analytics (churn/hotspots/ownership/coupling), and auto-generated per-file
documentation. Not (yet): ADR mining, an MCP server, or a dashboard — see
the root README for current scope.

## Boundaries
This is a straightforward layered pipeline, not a plugin-style
ports-and-adapters system with `dyn Trait` boundaries — worth saying plainly
rather than forcing that framing where it doesn't fit. The real seams are
match-based dispatch points, each with exactly one implementation per branch
today:

| Seam | Implementation(s) | Notes |
| ---- | ------------------ | ----- |
| Per-language extraction (`repowise_parser::parse_file`) | `rust::extract`, `python::extract` | callers match on `Language` and never touch per-language internals directly; adding a language means adding a match arm, not implementing a trait |
| File discovery (`repowise_core::discover_files`) | wraps the `ignore` crate | `.gitignore`-aware; the walker itself isn't swappable yet |
| Index persistence (`RepoIndex::save`/`load`) | JSON on disk (`.repowise/index.json`) | the one and only backing store so far |

## Structure
Modular monolith: one Cargo workspace, seven crates. Most are a layer over
the one below it — `repowise-core` (data model, file discovery, index
persistence) → `repowise-parser` (tree-sitter extraction) → `repowise-graph`
(dependency graph + queries) → `repowise-health` (deterministic scoring on
top of the graph) → `repowise-docs` (renders `repowise-health`'s findings
and `repowise-graph`'s deps/dependents into per-file pages) — all tied
together by `repowise-cli` (the binary). `repowise-git` is the one
exception to the strict pipeline: it depends only on `repowise-core` (for
`RepoIndex`'s per-symbol complexity) and the `git` CLI directly, not on
`repowise-graph`, since git-history analysis doesn't need the dependency
graph at all. No crate has been split out as a separate service or process;
there's no forcing function (scaling, team boundary, fault isolation) that
would justify it yet.

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

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs.

## Non-goals
- Compiler-grade name resolution — import/call resolution is directory-layout
  heuristics; ambiguous or external references are left unresolved rather
  than guessed (see README).
- Feature parity with the original repowise project's remaining
  intelligence layer (ADR mining), its MCP server, and its dashboard — not
  in scope until explicitly picked up.
