# Parity gaps vs. upstream repowise

This is a snapshot audit of where `rusty_repo_wise` (this Rust port) still
diverges from the original [repowise-dev/repowise](https://github.com/repowise-dev/repowise)
(Python/TypeScript, AGPL-3.0), as of 2026-07-31. It supersedes any earlier
in-conversation summary of the same comparison — an initial pass under-read
this repo's own README (3,143 lines) and missed several already-shipped
sections (`repowise-server`/`repowise-web`'s live dashboard, `repowise-distill`,
the post-commit hook, `repowise-workspace`'s later slices). That version was
cross-checked against this repo's own README/ARCHITECTURE.md and closed-issue
history, but not against upstream's own docs directly.

**Round 2 (2026-07-31) corrects that gap.** A direct read of upstream's
`docs/start/DASHBOARD.md` ("every view in the local web dashboard, and what
each one answers" — 23 views, listed by name) found this repo's own earlier
"the live dashboard is already at parity" claim below was true only relative
to this port's own issue #59/#65 bundle, not relative to upstream's actual,
current view list. Ten more gaps (issues #351–#360) came out of that direct
comparison — nine dashboard views this port doesn't have (some needing only
new wiring over data this port already computes; a few needing genuinely new
capabilities this port has never had), plus one non-dashboard capability gap
(security-finding scanning) surfaced by cross-referencing the view list
against this port's own capability inventory. See the table below; the
"already at parity" bullet immediately below is left as originally written,
with a note, rather than rewritten, so the correction itself stays visible.

Every open gap below has a matching GitHub issue labeled `parity-gap`
(alongside `enhancement`/`needs-human`, matching this repo's existing
convention from issues like #64 and #319). **Priority for this repo's
near-term work is parity** — closing these before starting anything not
tracked back to the upstream reference.

## Already at parity (not gaps)

Worth stating plainly, since the first pass got this wrong: the port has
reached genuine parity on the deterministic, index-only core, and in several
places exceeds upstream on raw count:

- All 16 of upstream's fully-parsed languages, plus its Structural (9
  languages) and Lightweight (6 languages) tiers, by name.
- 31 health markers (vs. upstream's headline ~25, which bundles the
  Performance-signal cluster as one item instead of listing each of the 19
  individually).
- All 8 of upstream's decision-mining sources, plus a 9th
  (README/ARCHITECTURE prose) upstream doesn't have.
- All ten of upstream's flagship MCP tools, plus three more
  (`list_repos`, `get_architecture`, `get_blast_radius`).
- A live dashboard server (`repowise serve-dashboard`, axum backend +
  `repowise-web` Leptos/WASM frontend) with JSON search, a dependency-graph
  view, chat backed by real embeddings retrieval, Present Mode, a live
  reindex job banner, a read-only Settings view, and in-process cost
  tracking — closing out issues #59 and #65's entire bundle.
  **Correction (Round 2, 2026-07-31): this was parity with this repo's own
  issue #59/#65 bundle, not with upstream's actual current dashboard.** A
  direct read of upstream's `docs/start/DASHBOARD.md` found 23 named views;
  this port has solid equivalents for roughly 15–16 of them and is missing
  or has only a partial equivalent for the rest — see issues #351–#360.
  The static-dashboard-vs-live-dashboard migration (this repo's own #59/#65
  scope) is genuinely done; parity with upstream's actual, current dashboard
  is not.
- `repowise-distill`: reversible command-output compaction with an
  omission store, a hook that routes recognized commands through it
  automatically, savings accounting, and fumble-correction detection
  (`repowise distill`/`expand`/`hook rewrite`/`saved`/`corrections`).
- A post-commit git hook (`repowise hook install`) and debounced
  file-watch re-indexing (`repowise watch`).
- Test intelligence: LCOV coverage ingest, impacted-tests, and
  `untested_hotspot`/`coverage_gap` health markers.
- `repowise-workspace`'s first four of #64's five bundled slices: repo
  listing, cross-repo co-change, cross-repo architecture/blast-radius,
  and a conformance/cycle-detection CI gate — plus contract
  producer/consumer matching (the fifth slice) and workspace-level
  propagation-cost/cyclic-core metrics.

## Deliberately declined, not gaps

Two upstream capabilities were reviewed by a human and closed as permanent
non-goals rather than deferred work — re-opening them isn't part of this
list:

- **ML-calibrated health-score weights and the change-risk regression
  model** (issue #62, #42): both need a labeled defect corpus and a
  training/versioning pipeline this port has never had infrastructure for.
  Fixed-penalty heuristics stand in for both permanently.
- **A hosted/SaaS offering and commercial dual-licensing**: a business-model
  decision, not an engineering gap — this port is MIT-licensed,
  self-hosted-only, and doesn't share code with upstream's AGPL-3.0 base.

## Open parity gaps (tracked as issues)

| # | Gap | Issue | Status |
|---|-----|-------|--------|
| 1 | VS Code extension | [#332](https://github.com/baileyrd/rusty_repo_wise/issues/332) | open |
| 2 | Claude Code / Codex / opencode agent plugins (hooks, skills, commands) | [#333](https://github.com/baileyrd/rusty_repo_wise/issues/333) | partially closed via PR #349 — `claude-plugin/` at repo root: bundles the MCP server, a `SessionStart` index-freshness hook, a `PreToolUse` Distill hook, and a skill; Claude Code only, Codex/opencode/PostToolUse/workspace-scoping left open, see the issue |
| 3 | GitHub PR bot | [#334](https://github.com/baileyrd/rusty_repo_wise/issues/334) | open |
| 4 | Webhook- and polling-triggered auto-sync | [#335](https://github.com/baileyrd/rusty_repo_wise/issues/335) | partially closed via PR #345 — `POST /api/webhook/github`/`/gitlab` added (both trigger the shared reindex job, `REPOWISE_WEBHOOK_SECRET`-gated); polling deliberately declined, see the issue |
| 5 | Native multi-provider LLM support in `repowise-llm` | [#336](https://github.com/baileyrd/rusty_repo_wise/issues/336) | closed, not planned — `rusty_provider` is the accepted permanent answer, see the issue |
| 6 | Federated workspace queries (`repo="all"`) | [#337](https://github.com/baileyrd/rusty_repo_wise/issues/337) | partially closed via PR #348 — `search_codebase`'s new `repo` parameter (named repo or `"all"`) federates lexical search across the MCP server's configured workspace; extending to other tools/the dashboard left open, see the issue |
| 7 | Cross-repo import resolution beyond Rust | [#338](https://github.com/baileyrd/rusty_repo_wise/issues/338) | closed via PR #343 — now covers Rust/Python/Java/Kotlin/Scala/Go/C#/PHP |
| 8 | Contract breaking-change detection | [#339](https://github.com/baileyrd/rusty_repo_wise/issues/339) | closed via PR #344 — persisted `.repowise-workspace/contracts.json` snapshot, diffed every `workspace-contracts` run |
| 9 | Git-worktree auto-seeding for incremental indexing | [#340](https://github.com/baileyrd/rusty_repo_wise/issues/340) | closed, not planned — this port has no incremental re-indexing at all yet, even for the common case; building it narrowly for worktrees first is the wrong order, see the issue |
| 10 | Luau ("Partial" tier) language support | [#341](https://github.com/baileyrd/rusty_repo_wise/issues/341) | closed via PR #347 — Luau joins the Full tier directly (real `tree-sitter-luau` grammar available), no new "Partial" tier concept needed; see `repowise_parser::luau`'s module doc |
| 11 | Dashboard: browsable Docs view + doc-freshness/coverage tracking | [#351](https://github.com/baileyrd/rusty_repo_wise/issues/351) | closed via PR #363 — `repowise_docs::check_freshness` (missing/fresh/stale, derived live from each page's own embedded content hash, no `docs` re-run needed) wired into a new `GET /api/doc-coverage`, `repowise doc-coverage`, and `get_doc_coverage` MCP tool, plus a new dashboard section (`#/docs`) listing every indexed file's status with a filter |
| 12 | Dashboard: Architecture section restructure (Map/Explore/Coupling sub-views) | [#352](https://github.com/baileyrd/rusty_repo_wise/issues/352) | closed via PR #364 (Coupling) + PR #368 (Map) — Coupling: `repowise-git`'s existing `top_co_changed_pairs` wired into `GET /api/coupling`/`repowise coupling`/`get_coupling`, plus a dashboard section (`#/coupling`); Map: new `repowise_graph::detect_communities` (multi-level Louvain), wired into `GET /api/communities` and a dashboard section (`#/map`) reusing the existing Files treemap layout, sized by lines of code; Explore was already at parity via the existing `/api/graph` |
| 13 | External (third-party) dependency registry | [#353](https://github.com/baileyrd/rusty_repo_wise/issues/353) | closed via PR #365 — new `repowise-external-deps` crate: declared (not lockfile-resolved) third-party deps from `Cargo.toml`/`package.json`/`composer.json`/`requirements.txt`/`pyproject.toml`/`go.mod`, wired into `repowise external-deps`, `GET /api/external-deps`, `get_external_deps`, and a new dashboard section (`#/dependencies`); Java/Kotlin/Scala/C#'s XML/Gradle-DSL manifests left for a follow-up |
| 14 | Dashboard: Knowledge Graph view | [#354](https://github.com/baileyrd/rusty_repo_wise/issues/354) | closed via PR #366 (module-grouping toggle) + PR #369 (Knowledge Graph section) — read upstream's docs directly: it's a full continuous-zoom canvas (repo→module→file→symbol) with a custom camera/culling renderer, not new backend data; PR #369 delivers the same repo→community→file→symbol hierarchy as a *semantic* zoom (click to drill in, breadcrumb to climb out) instead of a literal camera/culling renderer, reusing `/api/communities`/`/api/files`/`/api/symbols` (the last gaining `end_line`) filtered client-side rather than a new endpoint; deep-linking (`?focus=`) deliberately left for a follow-up, see the issue |
| 15 | Dashboard: Refactoring candidates view | [#355](https://github.com/baileyrd/rusty_repo_wise/issues/355) | closed via PR #362 — `GET /api/refactor-candidates` (optional `?kind=` filter, `total_matching`/20-candidate cap mirroring the MCP tool), a new dashboard section (`#/refactor-candidates`) with a kind filter and file drill-down links |
| 16 | Dashboard: Commits view (risk-scored commit browsing) | [#356](https://github.com/baileyrd/rusty_repo_wise/issues/356) | closed via PR #367 — `repowise-git` gained `collect_recent_commits` (bounded `git log -n`, not a full-history walk); wired into `repowise commits`, `GET /api/commits`, `get_commits`, and a new dashboard section (`#/commits`); risk scoring is on-demand per commit (`GET /api/commit-risk`, reusing `change_risk`) rather than eager for every listed commit, resolving both of the issue's own open questions |
| 17 | Dashboard: semantic search in the main search box | [#357](https://github.com/baileyrd/rusty_repo_wise/issues/357) | closed via PR #370 — new `GET /api/search-semantic` (reusing `POST /api/chat`'s own `repowise_llm::retrieve`), called by the search box only once `/api/search` has already come back empty for the settled query (automatic fallback, not an explicit mode toggle — resolves both of the issue's own open questions: no new UI, and single-repo only, matching `/api/search`'s own scope) |
| 18 | Dashboard: surface distill/MCP savings accounting (Costs view) | [#358](https://github.com/baileyrd/rusty_repo_wise/issues/358) | closed via PR #371 — new `GET /api/saved?by=program\|day`, reusing `repowise-distill`'s existing savings ledger rather than new accounting (mirrors `repowise saved`'s own measured-distillation/modelled-MCP/missed-commands split); a new dashboard Costs section (`#/costs`) with the same `by` grouping toggle the CLI offers — resolves the issue's own open question (its own view, matching upstream's separate framing, rather than folding into the existing Usage view) |
| 19 | Dashboard: configurable Settings view | [#359](https://github.com/baileyrd/rusty_repo_wise/issues/359) | open |
| 20 | Security-finding scanning + dashboard view | [#360](https://github.com/baileyrd/rusty_repo_wise/issues/360) | open |

See each linked issue for the gap/why/reference/open-questions detail —
this table is kept in sync with issue numbers once filed.
