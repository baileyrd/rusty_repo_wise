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

**Status as of 2026-08-06:** of the 20 tracked gaps below, 2 remain open
([#333](https://github.com/baileyrd/rusty_repo_wise/issues/333),
[#335](https://github.com/baileyrd/rusty_repo_wise/issues/335)) — both
partially closed, with the remainder needing a human decision.
[#337](https://github.com/baileyrd/rusty_repo_wise/issues/337) is now
fully closed. The other 18 are closed or closed-not-planned. Work shipped in the same period that
is *not* parity work is listed separately below, so this file isn't
mistaken for a full changelog.

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
  this port had solid equivalents for roughly 15–16 of them and was missing
  or had only a partial equivalent for the rest — tracked as issues
  #351–#360. **All ten are now closed** (see the table below), so the
  dashboard gap identified in Round 2 is worked through.
  The static-dashboard-vs-live-dashboard migration (this repo's own #59/#65
  scope) is genuinely done. It is also settled policy rather than an
  accident: issue #383 later proposed re-adding a static, committed-payload
  dashboard alongside the live one and was **closed as not planned**, on
  the grounds #59/#65 already established — keeping both means two
  dashboards to keep in sync on every future feature rather than one built
  well. See `docs/adr/0003-static-dashboard.md`, kept for the measurements
  even though the work was declined.
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

## Non-parity work shipped alongside (not gaps, not parity progress)

**2026-08-05.** A separate comparison — against
[Understand-Anything](https://github.com/Egonex-AI/Understand-Anything)
(TypeScript, LLM-agent-first), *not* upstream repowise — produced seven
issues and six shipped changes. **None of them are parity gaps**, none
carry the `parity-gap` label, and none appear in the table below. They
are listed here only so this file isn't read as a complete record of
what the port has gained.

| Shipped | Issue / PR |
|---------|------------|
| Guided tours: a deterministic, dependency-ordered reading path (`repowise tour`, `repowise-tour` crate) | #377 / PR #379 |
| Portable, committable index — repo-relative, canonically sorted, schema-versioned (`export --format index`, `--index <FILE>`) | #378 / PR #380, ADR-0002 |
| `--index` widened to all eleven index-derived read commands | #382 / PR #385 |
| Interned caller ids (portable schema v2, 18.3% smaller, lossless) | #381 / PR #386 |
| Portable-index-backed workspace members | #384 / PR #386 |
| Rust/Go module paths recorded in the artifact, so a never-cloned member still resolves cross-repo | #388 / PR #389 |
| Static committed-payload dashboard | #383 — **closed, not planned**; ADR-0003 kept |

**This deviated from the priority stated above.** That line reads
"closing these before starting anything not tracked back to the upstream
reference", and three parity gaps (#333, #335, #337) stayed open
throughout. #337 has since been closed; #333 and #335 remain open. Recorded rather than quietly absorbed, since the priority is
either real or it should be rewritten.

Two corrections these changes forced, worth keeping visible because they
were wrong in the issue text before they were right in the code:

- **Dropping `calls` from the index is not a compression option.** It
  takes `call_in_degree` to zero for every symbol, so the dead-code list
  contains *everything* and health scores collapse; dropping
  `field_accesses` fails the opposite way (LCOM4 goes quiet, scores
  silently rise). Interning was the lossless answer.
- **A static dashboard was already a settled non-goal**, decided by
  #59/#65 and documented in the README. #383 was filed from a
  feature-comparison without checking whether the absence was deliberate
  — a gap and a deliberate omission look identical from outside.

## Open parity gaps (tracked as issues)

| # | Gap | Issue | Status |
|---|-----|-------|--------|
| 1 | Zed extension (retargeted from VS Code — Zed extensions are Rust/WASM, a native fit for this workspace rather than a second TypeScript toolchain) | [#332](https://github.com/baileyrd/rusty_repo_wise/issues/332) | partially closed via PR #376 — `zed-extension/` at repo root registers `repowise serve` as a Zed context server (MCP server integration), the one upstream VS Code capability that maps cleanly to direct reuse of the existing `repowise-mcp` server; resolves all three of the issue's own open questions (confirmed Zed's real extension API against its own docs — Languages/Debuggers/Themes/Snippets/MCP Servers, no gutter/hover/CodeLens surface; MCP registration only for this first slice; lives in this repo at root, matching `claude-plugin/`'s precedent); not published to Zed's extension registry yet, see the issue |
| 2 | Claude Code / Codex / opencode agent plugins (hooks, skills, commands) | [#333](https://github.com/baileyrd/rusty_repo_wise/issues/333) | partially closed via PR #349 — `claude-plugin/` at repo root: bundles the MCP server, a `SessionStart` index-freshness hook, a `PreToolUse` Distill hook, and a skill; Claude Code only, Codex/opencode/PostToolUse/workspace-scoping left open, see the issue |
| 3 | GitHub PR bot | [#334](https://github.com/baileyrd/rusty_repo_wise/issues/334) | partially closed via PR #375 — `.github/actions/pr-risk-comment`, a self-hosted composite GitHub Action (not a hosted App — resolves the issue's own hosting-scope question) wrapping the existing `repowise risk` command as-is and posting/updating a single PR comment; dogfooded on this repo's own PRs via `.github/workflows/pr-risk-comment.yml`; health-delta and decisions-touched (upstream's other two PR-bot signals) left open, see the issue |
| 4 | Webhook- and polling-triggered auto-sync | [#335](https://github.com/baileyrd/rusty_repo_wise/issues/335) | partially closed via PR #345 — `POST /api/webhook/github`/`/gitlab` added (both trigger the shared reindex job, `REPOWISE_WEBHOOK_SECRET`-gated); polling deliberately declined, see the issue |
| 5 | Native multi-provider LLM support in `repowise-llm` | [#336](https://github.com/baileyrd/rusty_repo_wise/issues/336) | closed, not planned — `rusty_provider` is the accepted permanent answer, see the issue |
| 6 | Federated workspace queries (`repo="all"`) | [#337](https://github.com/baileyrd/rusty_repo_wise/issues/337) | **closed** — every MCP tool now takes `repo` or is workspace-level by construction, and every repo-scoped dashboard endpoint honours `?repo=`. `"all"` federates where rows can carry a repo label; where the answer is a single subject or an un-labeled list, `repo` selects which repo and `"all"` is refused with a reason rather than silently answering from one. Landed across PRs #348, #391, #393, #394, #399 and this round |
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
| 19 | Dashboard: configurable Settings view | [#359](https://github.com/baileyrd/rusty_repo_wise/issues/359) | closed via PR #372 — this port's first persisted, repo-level config file (`.repowise/config.toml`, `[health_weights]`-nested), a new `POST /api/settings/health-weights` (validates, persists, applied by `/api/health`/`/api/files` in place of the previously-hardcoded `HealthWeights::default()`), and a raw-TOML editor in the dashboard's existing Settings section — resolves all three of the issue's own open questions: health weights only for this first slice (existing schema/parser/CLI precedent, nothing security-sensitive, unlike webhook secrets); config lives in `.repowise/config.toml`; worth the scope since it stays file-based/inspectable rather than adding hidden state, matching the `--weights <FILE>` precedent it reuses |
| 20 | Security-finding scanning + dashboard view | [#360](https://github.com/baileyrd/rusty_repo_wise/issues/360) | closed via PR #373 — a direct read of upstream's docs found no dedicated security-scanning layer/doc (unlike every other capability), just a one-line "security findings table, by directory and by severity" dashboard sub-tab mention, and its own tool-comparison table concedes full SAST/SCA/secrets scanning to dedicated tools; new `repowise-security` crate does deterministic, signature-based hardcoded-secret detection only (AWS/GitHub/Slack tokens, PEM private-key blocks, a placeholder-filtered suspicious-assignment heuristic), wired into `repowise security`, `GET /api/security`, `get_security_findings`, and a new dashboard section (`#/security`); dependency-CVE checking and injection-shape detection deliberately declined (no live vulnerability-feed infra; needs real dataflow analysis this port doesn't have), resolving all three of the issue's own open questions |

See each linked issue for the gap/why/reference/open-questions detail —
this table is kept in sync with issue numbers once filed.
