---
name: repowise-overview
description: Use when working in a repo that has the repowise plugin enabled -- to know which repowise MCP tool or CLI command answers a given question (symbol/file search, per-file risk and health, cross-repo architecture, mined design decisions, coverage, dead code) and when to reach for it instead of Grep/Read/Bash.
---

This repo has the `repowise` codebase-intelligence tool available: an MCP
server (started automatically by this plugin) plus a `repowise` CLI. Both
answer from a persisted index (`.repowise/index.json`), not a live
filesystem walk, so they're fast -- but they need `repowise init`/`update`
to have run at least once. This plugin's `SessionStart` hook bootstraps
the index automatically on first use and reports whether an existing one
is stale; a stale-index warning also rides along on every individual MCP
tool response (`_meta.stale_warning`), so no single answer can look more
current than it is.

## Prefer these MCP tools over Grep/Read for the questions they answer directly

Each tool's own description (visible when the server is running) covers
its exact parameters -- this is about *when* to reach for one, not how to
call it.

- **Finding a symbol or file by name or path** -- `search_codebase`
  instead of `Grep`/`Glob`. Faster (index lookup, not a filesystem walk),
  and supports semantic search (`mode: "semantic"`, when an LLM endpoint
  is configured) for "where is X handled" questions a substring match
  can't answer. Also the one tool with cross-repo reach: pass
  `repo: "all"` to search every repo in a configured workspace at once,
  or `repo: "<name>"` for one specific repo, instead of just this
  server's own root.
- **Understanding one file before editing it** -- `get_context` instead
  of just `Read`. Returns the file's symbols, its dependencies/
  dependents in the import graph, and its health findings in one call --
  context `Read` alone doesn't give you.
- **Assessing risk before touching a file, or before a commit** --
  `get_risk` (hotspot/churn/health for a file, or the riskiest files
  repo-wide with no argument) and `get_change_risk` (diff-shape risk for
  a specific commit or range).
- **"Why does this code look like this?"** -- `get_why`, mined
  architectural decisions (ADRs, decision-flavored commits/PRs/comments,
  `WHY:`/`DECISION:` markers) linked to a file or symbol, each carrying a
  confidence score. Cheaper and more targeted than grepping commit
  history by hand.
- **Finding likely-dead code, or refactor candidates (god classes,
  duplicate logic, long parameter lists)** -- `get_dead_code` /
  `get_refactor_candidates`, both confidence-tiered rather than binary
  yes/no claims.
- **Multi-repo workspaces** (only if this server was started with
  `--workspace`) -- `list_repos`, `get_architecture` (cross-repo import
  graph), `get_blast_radius` (who else would need review if a file's
  public API changed).

None of these replace `Edit`/`Write` for making changes, or `Read` for
actually viewing a file's full content -- they're for finding *where* to
look and *what's already known* about it before you do.

## CLI escape hatches (not exposed via MCP)

A few things only exist as `repowise <command>` right now, not as an MCP
tool -- reach for `Bash` if one of these is what you need:

- `repowise decisions [--for-file <path>]` -- the full mined-decision
  list/detail view `get_why` summarizes; useful for a broader sweep than
  one file/symbol's linked decisions.
- `repowise coverage status` -- per-file test coverage after `repowise
  coverage add <lcov-report>` has ingested one.
- `repowise doctor` -- setup diagnostics (git availability, history
  depth, index presence, which optional env-var-gated features are
  active). Run this first if MCP tools are behaving unexpectedly.
- `repowise decide "<title>" "<rationale>"` -- record a new architectural
  decision, so future `get_why`/`decisions` calls (yours or a teammate's)
  can find it.

## What the plugin's own hooks do, so their output isn't a surprise

- **`SessionStart`** runs once per session: bootstraps the index if none
  exists, or reports freshness (never silently re-indexes an existing
  one -- that's `repowise update`'s job, left to you or the git
  post-commit hook if `repowise hook install` is set up).
- **`PreToolUse` (Bash only)** routes a small, closed set of recognized
  commands (`cargo test`, `pytest`, `npm run build`, and similar --
  see `repowise hook rewrite status` for the exact list) through
  `repowise distill` automatically, which trims routine, low-signal
  output (e.g. a passing test suite's own noise) before it reaches
  context. Anything outside that closed set, or containing shell syntax
  the hook doesn't parse, runs completely unmodified -- this hook never
  blocks a command, only rewrites a known-safe subset of them.
