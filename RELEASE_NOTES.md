# Release Notes

Notable changes to this repo, newest first. No tagged releases yet, so entries
are keyed by commit (this repo hasn't routed changes through PRs until now —
future entries will link PRs once that's the norm, per CONTRIBUTING.md).

---

## 2026-07-22 — Add deterministic code-health scoring layer
[`088dad1`](https://github.com/baileyrd/rusty_repo_wise/commit/088dad137b8cca871f1aeaf671a46e6776e81b35)

- **Added:** a new `repowise-health` crate scoring every indexed file 0–10 from
  six deterministic markers — long functions, high cyclomatic complexity,
  oversized parameter lists, god classes, duplicate code, and possibly-dead
  code — wired up as `repowise health [PATH]`. `repowise-parser` now computes
  per-function cyclomatic complexity, parameter count, and a duplicate-code
  body hash; `repowise-graph` gained `call_in_degree()` to support the
  dead-code check.
- **Known limitation, stated plainly:** covers 6 of the original repowise's
  ~25 health markers. Git-history-based markers (churn/hotspots, ownership,
  co-change coupling), LCOM4 cohesion, and substring-level (Rabin-Karp) clone
  detection are deferred — this only detects whole-function-body duplicates
  via exact hash match, not partial/near-duplicate code.
- 9 new tests (5 health-scoring, 4 parser); 15 tests passing workspace-wide.

## 2026-07-22 — Scaffold Rust port of repowise: dependency-graph layer + CLI
[`1d45806`](https://github.com/baileyrd/rusty_repo_wise/commit/1d458060e72fc33b001cf8800a57d0e90d35874c)

- **Added:** initial Rust workspace (`repowise-core`, `repowise-parser`,
  `repowise-graph`, `repowise-cli`) implementing the dependency-graph
  intelligence layer from repowise: tree-sitter extraction of symbols,
  imports, and calls for Rust and Python, a petgraph-backed dependency graph
  with directory-layout-based import/call resolution, and
  `init`/`update`/`overview`/`search`/`deps` CLI commands.
- **Known limitation, stated plainly:** import/call resolution is heuristic
  (directory-layout conventions), not full compiler-grade name resolution —
  ambiguous or external references are left unresolved rather than guessed.
  Only Rust and Python are parsed; repowise's other 14 languages aren't
  implemented. Git analytics, doc generation, ADR mining, the MCP server, and
  the web dashboard are out of scope for now.
- 6 tests passing (2 graph integration tests, 4 parser unit tests).
