# Release Notes

Notable changes to this repo, newest first. No tagged releases yet, so entries
are keyed by PR (or by commit, for the two prior changes that predate this
repo routing work through PRs).

---

## PR #73 — Add recency-weighted hotspot scoring
**2026-07-23** · [#73](https://github.com/baileyrd/rusty_repo_wise/pull/73) · closes [#28](https://github.com/baileyrd/rusty_repo_wise/issues/28)

- **Added:** `repowise-git`'s `hotspots()` now ranks files by a
  recency-weighted variant of churn × complexity — each commit
  contributes `exp(-age_days / 90)` toward a file's "decayed churn"
  instead of a flat `1`, so a file touched recently outranks an
  equally-churny file that's gone quiet. `CommitInfo` gained an
  author-date timestamp field to support this. The existing raw
  `score`/`churn` fields are unchanged and still reported (CLI, dashboard)
  alongside the new `decayed_score`.
- **Known limitation, stated plainly:** the decay half-life (90 days) is a
  fixed, documented constant, not configurable — no per-repo tuning.
  The bug-fix heuristic (message keywords only, no linked-issue-reference
  signal) remains a separate, still-open gap (issue #60).
- 1 new test (verifying both the decayed-score values and the resulting
  rank order — a recently-touched file outranks an equally-churny old
  one); 45 tests passing workspace-wide. First PR merged out of this
  session's `parity-loop` gap-analysis pass against the real repowise
  (45 gap issues filed: #28, #29-40 languages, #41-45 MCP tools, #46-50
  ADR sources, #51-56 health markers, #57-59 dashboard, #60 git analytics,
  plus 13 `needs-human` issues for product-direction/design questions
  outside the loop's auto-implement scope).

## PR #26 — Add TypeScript/JavaScript language support
**2026-07-23** · [#26](https://github.com/baileyrd/rusty_repo_wise/pull/26) · closes [#22](https://github.com/baileyrd/rusty_repo_wise/issues/22)

- **Added:** a `repowise-parser` extractor for TypeScript and JavaScript —
  functions, methods, classes, TypeScript interfaces (mapped to `Trait`),
  and named arrow-function/function-expression bindings, each with
  cyclomatic complexity, parameter count, and a duplicate-code body hash.
  ESM `import`/CommonJS `require` are resolved directly against the
  filesystem for relative (`./`, `../`) specifiers, same as Rust's
  `mod foo;`. `new ClassName(...)` is recorded as a call to the class so
  instantiated classes don't read as dead code. Bumped `tree-sitter` and
  the Rust/Python grammar crates to `LanguageFn`-based versions so
  `tree-sitter-typescript`/`tree-sitter-javascript` share the same core.
- **Known limitation, stated plainly:** no `node_modules` resolution
  (bare/npm specifiers are left unresolved) and no `tsconfig.json` path
  alias support — both explicitly out of scope per issue #22. A `new
  ClassName()` call resolves to the class, not the constructor method
  itself, so an only-ever-`new`'d class's constructor can still read as
  possibly-dead-code — a known heuristic gap, not a bug.
- 9 new tests (symbol/class/interface extraction, arrow/function-expression
  bindings, ESM+CommonJS imports, `new`-expression call tracking, cyclomatic
  complexity, duplicate-body hashing) plus a `repowise-graph` end-to-end
  relative-import-resolution test; 44 tests passing workspace-wide. Also
  filed issues #23 (MCP `get_risk`/`get_change_risk`), #24 (dashboard live
  search/drill-down/auto-refresh), and #25 (ADR mining additional sources)
  covering the other previously-called-out known limitations.

## PR #20 — Add static-site dashboard
**2026-07-23** · [#20](https://github.com/baileyrd/rusty_repo_wise/pull/20) · closes [#10](https://github.com/baileyrd/rusty_repo_wise/issues/10)

- **Added:** a new `repowise-dashboard` crate rendering a single
  self-contained HTML page — overview, code health, hotspots, and
  architectural decisions — from data the other layers already compute.
  No server, no JS build step, no templating engine; regenerate with
  `repowise dashboard [PATH]` and open `.repowise/dashboard/index.html`
  in a browser. All user-controlled text (language labels, file paths,
  decision titles) is HTML-escaped.
- **Known limitation, stated plainly:** static only — no live search,
  no per-file drill-down, no auto-refresh; re-run the command after the
  repo changes. Git-history and ADR/decision data degrade gracefully to
  explicit "not available" placeholders when a root has no git history
  or no ADRs, rather than failing the whole generation.
- 4 new tests (escaping, relative-path rendering with placeholders,
  hotspots/decisions rendering, an end-to-end real-index integration
  test); 36 tests passing workspace-wide. With this, all five of
  repowise's original "intelligence layers," its MCP server, and a
  dashboard have at least partial implementations in this port.

## PR #18 — Add MCP server: get_overview, search_codebase, get_context
**2026-07-22** · [#18](https://github.com/baileyrd/rusty_repo_wise/pull/18) · closes [#9](https://github.com/baileyrd/rusty_repo_wise/issues/9)

- **Added:** a new `repowise-mcp` crate exposing `get_overview`,
  `search_codebase`, and `get_context` as MCP tools over stdio via the
  official `rmcp` SDK, wired up as `repowise serve [PATH]`. `get_context`
  bundles a file's symbols, resolved deps/dependents, and health
  score/findings into one call — the tool that matters most for the
  original's "cut agent token spend on context-loading" goal.
- **Known limitation, stated plainly:** `get_risk`/`get_change_risk` are
  deferred to a follow-up rather than bundled in — they'd read naturally
  on `repowise-git`'s hotspot data. No caching across tool calls (same
  choice already made for `hotspots`/`ownership`/`coupled`/`decisions`).
- Verified the `rmcp` API against the actual installed crate's own
  doctests before writing real code — a fetched README described an
  older major version that didn't match what `cargo add` resolves.
- 5 new tests calling each tool method against a real index built by the
  actual indexing pipeline; 32 tests passing workspace-wide. With this,
  all five of repowise's original "intelligence layers" plus its MCP
  server have at least partial implementations in this port — only the
  web dashboard remains unstarted.

## PR #16 — Add architectural-decision (ADR) mining layer
**2026-07-22** · [#16](https://github.com/baileyrd/rusty_repo_wise/pull/16) · closes [#8](https://github.com/baileyrd/rusty_repo_wise/issues/8)

- **Added:** a new `repowise-adr` crate mining decisions from `docs/adr/*.md`
  files and decision-like commit messages (keyword heuristic), linking each
  to the files/symbols its body mentions, and tracking supersession via an
  ADR's existing `Status: Superseded by ADR-XXXX` line (no new front-matter
  convention needed). Wired up as `repowise decisions [PATH] [--for-file <FILE>]`.
  `repowise-git` gained `collect_commits()` so this reuses its git-log
  parsing instead of duplicating it.
- **Known limitation, stated plainly:** only 2 of the original repowise's 8
  decision sources are implemented (ADR files, commit messages) — PR
  descriptions, code comments, and integrations this repo doesn't have
  (Slack, issue trackers) are not mined. Linking is text matching, not
  semantic — a decision that only refers to a file descriptively won't
  be linked.
- 6 new tests (ADR parsing, unfilled-template skip, decision-commit
  detection, file/symbol linking, an end-to-end real-git-repo test); 27
  tests passing workspace-wide. With this, all five of repowise's
  original "intelligence layers" have CLI-facing implementations in this
  port (each covering a subset of the original's scope per layer).

## PR #14 — Add auto-generated documentation layer: per-file wiki pages
**2026-07-22** · [#14](https://github.com/baileyrd/rusty_repo_wise/pull/14) · closes [#7](https://github.com/baileyrd/rusty_repo_wise/issues/7)

- **Added:** a new `repowise-docs` crate rendering one deterministic
  markdown page per indexed file under `.repowise/wiki/`: symbol list,
  resolved dependencies/dependents (`repowise-graph`), and health
  findings (`repowise-health`). No LLM involved. Wired up as
  `repowise docs [PATH]`.
- **Known limitation, stated plainly:** freshness (new/changed/unchanged)
  is tracked via a hash of each file's *own* source only — a page can
  report "unchanged" while its rendered content actually differs, if
  what changed was cross-file data (a new caller, a health finding from
  another file). Pages are always rewritten with current data regardless
  of the reported status, so content itself is never stale, only the
  status label can undersell how much changed.
- 2 new tests (a `render_page` unit test, a real-directory integration
  test for the New/Changed/Unchanged transitions); 21 tests passing
  workspace-wide.

## PR #12 — Add git-analytics layer: churn, hotspots, ownership, co-change coupling
**2026-07-22** · [#12](https://github.com/baileyrd/rusty_repo_wise/pull/12) · closes [#6](https://github.com/baileyrd/rusty_repo_wise/issues/6)

- **Added:** a new `repowise-git` crate computing git-history analytics by
  shelling out to `git log`/`git blame` — per-file churn, hotspot score
  (churn × total cyclomatic complexity, reusing `repowise-parser`'s
  existing complexity data), bug-fix-commit frequency (message-keyword
  heuristic), co-change coupling, and per-author line ownership. Wired
  up as `repowise hotspots`/`ownership`/`coupled`.
- **Known limitation, stated plainly:** git analytics are computed fresh
  on every invocation rather than cached in `.repowise/index.json`, to
  avoid taking on cache-invalidation design in this pass. Bug-fix
  detection is a message-keyword heuristic, not linked-issue-aware.
- 4 new integration tests build real, disposable git repos (via the
  `git` CLI) to exercise this against actual `git log`/`git blame`
  output; 19 tests passing workspace-wide.

## PR #4 — Update default-branch references now that main exists
**2026-07-22** · [#4](https://github.com/baileyrd/rusty_repo_wise/pull/4)

- **Changed:** the repo's default branch was renamed on GitHub from
  `claude/repowise-rust-port-pcxhal` to `main`. Updated the two places that
  hardcoded the old name: `CLAUDE.md`'s workflow description and
  `ci-rust.yml`'s `push` trigger (previously pinned to the old branch name
  with a comment to update it once `main` existed).
- Earlier entries in this file that mention the old branch name describe
  the state at the time those changes were made and are left as an
  accurate historical record rather than rewritten.

## PR #1 — Add standard governance files (PR/issue templates, docs, CI)
**2026-07-22** · [#1](https://github.com/baileyrd/rusty_repo_wise/pull/1)

- **Added:** the standard governance-file set — PR/issue templates,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, this file, ARCHITECTURE
  (hand-adapted to this repo's actual crate layout), an ADR seed, and a Rust
  CI workflow (fmt + clippy + test) gating merges going forward.
- **Fixed:** three pre-existing `clippy::unnecessary_sort_by` lints (two in
  `repowise-graph`, one in `repowise-health`) that the new CI caught — this
  repo had never run clippy in CI before, so they'd gone unnoticed locally
  under an older clippy version.
- **Known limitation, stated plainly:** the repo's GitHub Actions "allowed
  actions" policy initially blocked `actions/checkout`/`actions/cache`
  entirely (first-party actions), which had to be fixed in repo settings
  before CI could run at all — not something a workflow-file change alone
  could fix. Also: `ci-rust.yml` triggers on pushes to
  `claude/repowise-rust-port-pcxhal` specifically since there's no `main`
  yet; update that trigger once a conventional default branch exists.

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
