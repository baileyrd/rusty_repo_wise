# Architecture

## Overview
A Rust CLI (`repowise`) that indexes a codebase and answers questions about
it: dependency graph queries and deterministic code-health scores. Not (yet):
git-history analytics, doc generation, ADR mining, an MCP server, or a
dashboard — see the root README for current scope.

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
Modular monolith: one Cargo workspace, five crates, each a layer over the one
below it — `repowise-core` (data model, file discovery, index persistence) →
`repowise-parser` (tree-sitter extraction) → `repowise-graph` (dependency
graph + queries) → `repowise-health` (deterministic scoring on top of the
graph) → `repowise-cli` (binary tying it together). No crate has been split
out as a separate service or process; there's no forcing function (scaling,
team boundary, fault isolation) that would justify it yet.

## Data flow
`init`/`update` → `discover_files` walks the tree → `repowise_parser::parse_file`
extracts symbols/imports/calls per file into a `RepoIndex` → saved to
`.repowise/index.json`. Every other command (`overview`, `search`, `deps`,
`health`) loads that index, builds a `RepoGraph` (resolves imports/calls into
`Contains`/`Imports`/`Calls` edges), and queries it — `repowise-health` adds
one more pass over the graph's symbols and call-in-degrees to score files.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs.

## Non-goals
- Compiler-grade name resolution — import/call resolution is directory-layout
  heuristics; ambiguous or external references are left unresolved rather
  than guessed (see README).
- Feature parity with the original repowise project's other four
  intelligence layers, its MCP server, and its dashboard — not in scope until
  explicitly picked up.
