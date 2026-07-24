# Release Notes

Notable changes to this repo, newest first. No tagged releases yet, so entries
are keyed by PR (or by commit, for the two prior changes that predate this
repo routing work through PRs).

---

## PR #149 — Add ownership, dead-code, and decision-tracker views (Phase 4)
**2026-07-24** · [#149](https://github.com/baileyrd/rusty_repo_wise/pull/149) · part of [#59](https://github.com/baileyrd/rusty_repo_wise/issues/59) and [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** Phase 4 of the dashboard-server pivot — the last
  non-LLM-dependent views. Only the chat view (tying into #61's
  remaining LLM follow-ups) is left after this.
- **New `repowise-server` endpoints:** `GET /api/ownership?path=<rel>`
  (one file's git-blame author breakdown, `{"available": false}` for a
  non-git-repo root or unindexed path); `GET /api/dead-code`
  (confidence-tiered dead-code candidates with an optional
  `?min_confidence=low|medium|high` filter, mirroring the
  `get_dead_code` MCP tool's own shape). `/api/decisions` now takes an
  optional `?file=<rel>` to filter to decisions linked to one file (a
  per-file decision tracker); omitted, it behaves exactly as before.
- **`repowise-web`:** broadens every file-path drill-down from Phase
  2's wiki-only gating into a **file-detail panel** — wiki content,
  ownership breakdown, and linked decisions, each loading and failing
  independently, so a file with no wiki page yet still shows whatever
  ownership/decision data exists instead of one shared error. Every
  indexed file is clickable now, not just ones with a wiki page. A new
  **dead-code section** lists candidates with a minimum-confidence
  filter; each risk factor is available as a tooltip on the confidence
  cell.
- Verified end-to-end manually: built a scratch git repo with a
  decision-comment above a deliberately-uncalled function, confirmed
  all three new/changed endpoints' JSON, then drove the live page with
  headless Chromium — confirmed the file-detail panel shows wiki +
  ownership + linked decisions together, confirmed Close unmounts it,
  and confirmed the dead-code section's own file link reopens the same
  panel.
- **Fixed (CI):** the `/api/ownership` test fixture's `git commit`
  relied on this sandbox's ambient git identity, which doesn't exist on
  the CI runner ("empty ident name") — fixed by setting local
  `user.name`/`user.email` config explicitly in the test helper.
- **Scope:** still not full parity. Chat (tying into #61's remaining
  LLM follow-ups) is the one view left, not done here. This PR does not
  close #59 or #65.

---

## PR #147 — Add dependency-graph view (Phase 3)
**2026-07-24** · [#147](https://github.com/baileyrd/rusty_repo_wise/pull/147) · part of [#59](https://github.com/baileyrd/rusty_repo_wise/issues/59) and [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** Phase 3 of the dashboard-server pivot — a visual
  dependency-graph view, the last major static-dashboard-parity piece
  that hadn't been ported yet.
- **New `repowise-server` endpoint:** `GET /api/graph` — the file-level
  import graph as `{nodes, edges, truncated}`. Truncated to the 150
  most-connected files (ranked by dependency + dependent count) so the
  view stays renderable on a large repo; `truncated: true` signals when
  that cap actually cut something, rather than silently showing a
  partial graph that looks complete.
- **`repowise-web`:** a new graph section renders `/api/graph` as SVG,
  laid out client-side with a hand-rolled Fruchterman-Reingold-style
  force-directed simulation (deterministic circular start, all-pairs
  repulsion, edges as springs, gentle pull to center) — no D3 or other
  JS graph library, keeping the whole frontend buildable with just
  `cargo`/`trunk`. Nodes are colored by language using GitHub's own
  per-language colors; clicking a node opens its wiki page inline, the
  same drill-down convention every other section uses.
- Verified end-to-end manually: built a 4-file scratch Rust crate with
  a real `mod`-based import chain, confirmed `/api/graph`'s JSON, then
  drove the live page with headless Chromium — confirmed the SVG's
  node/edge counts, took a full-page screenshot to visually confirm a
  sane layout, and confirmed clicking a node opens its wiki page.
- **Scope:** still not full parity. Ownership/dead-code/decision-
  tracker views and chat (tying into #61's remaining LLM follow-ups)
  are later phases, not done here. This PR does not close #59 or #65.

---

## PR #145 — Add wiki drill-down links and instant search (Phase 2)
**2026-07-24** · [#145](https://github.com/baileyrd/rusty_repo_wise/pull/145) · part of [#59](https://github.com/baileyrd/rusty_repo_wise/issues/59) and [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** Phase 2 of the dashboard-server pivot — wiki-page
  drill-down links and instant search, the two remaining pieces of the
  static dashboard's UX Phase 1 hadn't ported yet.
- **New `repowise-server` endpoints:** `GET /api/wiki-pages` (which
  indexed files already have a `repowise-docs` wiki page on disk),
  `GET /api/wiki?path=<rel>` (serves one page's raw markdown, matched
  against that exact set rather than joined onto the root directly, so
  a crafted `path` can't escape `.repowise/wiki/` via `..` segments),
  and `GET /api/search?q=<term>` (case-insensitive substring match over
  file paths and symbol names, capped at 20 results each).
- **`repowise-web`:** every rendered file path (overview, health,
  hotspots, symbols) is now a drill-down link that opens its wiki page
  inline as raw markdown when one exists. A new Ctrl/Cmd+K-focusable
  search box live-queries `/api/search` as you type.
- **Fixed:** the overview section's "Most depended-on files" table —
  present in the static dashboard, computed by `/api/overview` since
  Phase 0, but never actually rendered by `repowise-web` until now.
- Verified end-to-end manually: generated a real wiki page with
  `repowise docs`, then drove the live page with headless Chromium —
  clicked a drill-down link and confirmed the markdown rendered inline,
  confirmed Close unmounts it, typed into the search box and confirmed
  live results, and confirmed Ctrl+K focuses the search input.
- **Scope:** still not full parity. A dependency-graph view, and
  eventually ownership/dead-code/decision-tracker views + chat, are
  later phases, not done here. This PR does not close #59 or #65.

---

## PR #143 — Port health/hotspots/decisions/symbols views (Phase 1)
**2026-07-23** · [#143](https://github.com/baileyrd/rusty_repo_wise/pull/143) · part of [#59](https://github.com/baileyrd/rusty_repo_wise/issues/59) and [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** Phase 1 of the dashboard-server pivot — porting every view
  the static `repowise dashboard` page already had onto Phase 0's live
  JSON API and Leptos frontend.
- **New `repowise-server` endpoints:** `GET /api/health`,
  `/api/hotspots`, `/api/decisions`, `/api/symbols`, mirroring the
  static dashboard's own sections (`repowise health`/`hotspots`/
  `decisions`, plus the full symbol list). `/api/hotspots` returns
  `{"available": false}` (not an error) when the root has no git
  history, same graceful-degradation behavior as the static page.
- **Fixed:** `/api/overview`'s `most_depended_on` now returns paths
  relative to the indexed root, like every other endpoint — Phase 0
  had it leaking this host's absolute filesystem layout.
- **`repowise-web`:** now renders all five sections, each fetching and
  suspending independently so one slow/failing section doesn't block
  the rest. The symbols table's kind filter is a real Leptos signal
  (client-side-reactive), not the static dashboard's embedded
  vanilla-JS toggle.
- Verified end-to-end manually: indexed a scratch git repo with a
  deliberately complex function, ran `trunk build` + the real compiled
  server, `curl`'d all five endpoints for correct relative-path JSON,
  then loaded the page in headless Chromium and confirmed every
  section renders real data with no console errors.
- **Scope:** still not full parity. Rendered file paths don't drill
  down to their `repowise-docs` wiki pages yet (the static dashboard's
  do); instant/Cmd+K search, a dependency-graph view, and
  ownership/dead-code/decision-tracker views + chat are later phases,
  not done here. This PR does not close #59 or #65.

---

## PR #141 — Add live dashboard server scaffolding (Phase 0)
**2026-07-23** · [#141](https://github.com/baileyrd/rusty_repo_wise/pull/141) · part of [#59](https://github.com/baileyrd/rusty_repo_wise/issues/59) and [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** Phase 0 of the pivot away from the one-shot static
  `repowise dashboard` page toward a genuinely live server, matching
  real repowise's Next.js+FastAPI architecture instead of a static
  site (issue #10's original design).
- **New `repowise-server` crate:** an `axum` backend exposing indexed
  repo data as JSON. Only `GET /api/overview` exists so far — proving
  the server-plus-frontend architecture end to end. It also serves a
  built frontend's static assets via `ServeDir` when `--static-dir` is
  given.
- **New `repowise-web` crate:** a Leptos (WASM, CSR) frontend that
  fetches `/api/overview` and renders it. Deliberately kept as its own
  standalone Cargo workspace (an empty `[workspace]` table in its
  `Cargo.toml`) rather than a member of the root workspace, so its
  WASM-only target can never break `cargo build/test/clippy
  --workspace` for the rest of the project. Build it with `cd
  crates/repowise-web && trunk build`.
- **New `repowise serve-dashboard [PATH]` CLI command** (`--addr`,
  `--static-dir`) — starts the live server, printing a hint about
  building the frontend if `--static-dir` is omitted.
- Verified end-to-end manually: indexed a scratch repo, ran `trunk
  build`, started the real compiled server pointed at the built
  `dist/`, and `curl`-confirmed both the JSON API and static asset
  serving (including correct `content-type: application/wasm`).
- **Scope:** this is scaffolding only — one endpoint, no ported views.
  Porting the existing static-dashboard views (health, hotspots,
  decisions, symbols) onto this same JSON-API shape is the next phase,
  not done here. This PR does not close #59 or #65.

---

## PR #139 — Abstract health-scoring penalty weights via HealthWeights
**2026-07-23** · [#139](https://github.com/baileyrd/rusty_repo_wise/pull/139) · part of [#62](https://github.com/baileyrd/rusty_repo_wise/issues/62)

- **Added:** `repowise_health::HealthWeights`, a precursor abstraction
  for #62's ML-calibrated health-score weights. `Default` matches this
  crate's original hand-picked penalties exactly, so every existing
  caller (`repowise health`/`docs`/`dashboard`, the MCP server) sees no
  behavior change.
- `HealthWeights::from_toml_str()` parses a (possibly partial) TOML
  document; an omitted key falls back to its documented default, so a
  custom weights file only needs to name the penalties it wants to
  change.
- New `analyze_with_weights(index, graph, weights)` is the customizable
  entry point; `analyze(index, graph)` stays exactly as before,
  delegating to it with `HealthWeights::default()`.
- New `repowise health --weights <FILE>` CLI flag — the first real
  consumer of the abstraction.
- **Scope:** this is plumbing, not calibration itself. A real
  calibrated weight set still needs a labeled defect corpus and a
  training pipeline this port doesn't have — that sourcing question
  remains open. This PR does not close #62.

---

## PR #137 — Add opt-in LLM-written wiki summaries via repowise-llm
**2026-07-23** · [#137](https://github.com/baileyrd/rusty_repo_wise/pull/137) · part of [#61](https://github.com/baileyrd/rusty_repo_wise/issues/61)

- **Added:** a first, deliberately narrow slice of issue #61's
  LLM-dependent feature tier. New `repowise generate [PATH]` CLI
  command layers an LLM-written summary on top of each existing
  `repowise-docs` wiki page.
- **New `repowise-llm` crate:** an OpenAI-compatible chat-completions
  client (`ureq`, synchronous — same HTTP-client choice
  `repowise-adr`/`repowise-git` already made for their own opt-in
  network calls, so `repowise generate` doesn't need an async runtime
  the way `repowise serve` does). Works against any OpenAI-compatible
  endpoint, including a self-hosted
  [`rusty_provider`](https://github.com/baileyrd/rusty_provider)
  instance.
- **Entirely opt-in**, mirroring `REPOWISE_GITHUB_TOKEN`'s "unset =
  feature off" pattern: `REPOWISE_LLM_BASE_URL` is the on/off switch,
  with `REPOWISE_LLM_MODEL` (default `"smart"`) and an optional
  `REPOWISE_LLM_API_KEY`.
- `generate_wiki_summaries` reads each file's existing wiki page (via a
  new `pub wiki_page_path` helper exported from `repowise-docs`), asks
  the LLM for a 2-3 sentence summary, and inserts it as a "## Summary"
  section right after the title — idempotent (replaces rather than
  stacks a previous summary) and per-file fault-tolerant (one page's
  failure doesn't stop the rest; `repowise generate` reports
  written/skipped/failed counts).
- Requires `repowise docs` to have been run first — same "augment,
  don't generate" relationship the dashboard drill-down links (#57)
  have with wiki pages.
- Tests: `complete()` against a real HTTP/JSON fixture server (same
  hand-rolled approach `repowise-adr`/`repowise-git`'s own network-call
  tests use), `insert_summary`'s idempotent-replace behavior, and an
  end-to-end `generate_wiki_summaries` test covering both an existing
  wiki page and a missing one.
- **Scope:** wiki-prose generation only. RAG chat, refactor-plan
  codegen, and doc-gen-as-decision-source — the other three features
  #61 bundles — remain deferred as separate follow-ups needing their
  own retrieval/context design. This PR does not close #61.

---

## PR #135 — Add linked-issue-reference bug-fix heuristic
**2026-07-23** · [#135](https://github.com/baileyrd/rusty_repo_wise/pull/135) · closes [#60](https://github.com/baileyrd/rusty_repo_wise/issues/60)

- **Added:** a GitHub-issue-reference-based bug-fix heuristic,
  complementing (not replacing) the existing message-keyword one. A
  commit now counts as a bug fix if its message contains a keyword
  (`fix`/`bug`/`hotfix`/`patch`) **or** references a GitHub issue
  (`#123`) that's closed with a bug-like label — a union.
- **New `repowise-git::issue_refs` module:** `parse_issue_refs` extracts
  `#N` references from a commit message (rejecting markdown headers,
  hex-color-like tokens, and `#` glued onto a preceding identifier);
  `is_closed_bug_issue` queries the GitHub API for an issue's
  closed/label state; `parse_github_owner_repo` parses a git remote URL
  (a near-duplicate of `repowise-adr`'s own copy — not shared
  cross-crate since `repowise-adr` already depends on `repowise-git`,
  not the reverse).
- **Opt-in, same pattern as `repowise-adr`'s PR-body decision source:**
  the linked-issue check only runs behind a `REPOWISE_GITHUB_TOKEN`
  environment variable; no token, no GitHub-hosted `origin` remote, or
  a failed lookup all degrade to keyword-only detection rather than
  failing `GitAnalytics::collect()`.
- `repowise-git` gains `ureq`/`serde`/`serde_json` dependencies (same
  versions `repowise-adr` already pins).
- Tests: `parse_issue_refs` edge cases, `is_closed_bug_issue` against a
  real HTTP/JSON fixture server (same hand-rolled approach
  `repowise-adr`'s `pull_requests` tests use), and
  `linked_bugfix_issue_numbers`'s degradation paths (no token / no
  remote / non-GitHub remote) against real disposable git repos.
- This closes out the git-analytics parity-gap issue (#60), the last
  known mechanical (non-`needs-human`) issue in the current backlog.

---

## PR #133 — Add dashboard symbols index section with a kind filter
**2026-07-23** · [#133](https://github.com/baileyrd/rusty_repo_wise/pull/133) · closes [#58](https://github.com/baileyrd/rusty_repo_wise/issues/58)

- **Added:** a new "Symbols" section in the dashboard — a table of every
  indexed symbol (name, kind, file, line), with a small embedded-JS
  dropdown that filters the table by kind (function/method/class/etc.)
  client-side.
- No external requests, no build step: the whole table is embedded once
  in the page; the dropdown just toggles row visibility via a
  `data-kind` attribute per row. This is the only JS anywhere in the
  dashboard.
- File cells reuse the drill-down linking added in #57 (`file_cell()`),
  so a symbol's file links to its wiki page when one exists on disk.
- `render()` gains a `RepoIndex` parameter (all call sites updated)
  since per-symbol data lives on `Symbol`/`FileRecord`, not the
  `Overview`/`HealthReport` data the other sections already consume.
- Tests: a render-level unit test covering the table/filter markup and
  wiki-page linking, plus an end-to-end `generate()` assertion that the
  real pipeline's indexed symbol shows up in the rendered table.
- This is the second of the three dashboard parity-gap issues (#57-59);
  #59 (live/instant search) is `needs-human` since it needs a design
  decision before it's implementable.

---

## PR #131 — Add dashboard per-file drill-down links to wiki pages
**2026-07-23** · [#131](https://github.com/baileyrd/rusty_repo_wise/pull/131) · closes [#57](https://github.com/baileyrd/rusty_repo_wise/issues/57)

- **Added:** every file path rendered in the dashboard's overview/health/
  hotspots tables (Most depended-on files, Lowest-scoring files,
  Hotspots) now links to that file's `repowise-docs` wiki page
  (`.repowise/wiki/<path>.md`) when one already exists on disk.
- **Scope decision:** the issue left open whether `dashboard` should
  require `docs` to have been run first and link if present, or
  generate wiki-page-equivalents itself. Went with **"check disk, link
  if present"** — `repowise-dashboard` doesn't gain a `repowise-docs`
  dependency, doesn't duplicate its freshness-tracking logic, and
  doesn't re-read every file from disk on every dashboard build.
  Drill-down links only appear after `repowise docs` has been run at
  least once; missing wiki pages degrade to plain (still-escaped) text,
  never a broken link.
- `generate()` computes a `HashSet<PathBuf>` of files with an existing
  wiki page on disk and passes it to `render()`, which stays pure (no
  filesystem access of its own). New `file_cell()` helper renders a
  link or plain text depending on set membership.
- The Architectural decisions table is unaffected — its rows are
  decisions, not files.
- Tests: a render-level unit test covering both branches (linked vs.
  plain text), and an end-to-end `generate()` test that regenerates the
  dashboard before and after a wiki page appears on disk.

---

## PR #129 — Add primitive_obsession param-type health marker
**2026-07-23** · [#129](https://github.com/baileyrd/rusty_repo_wise/pull/129) · closes [#56](https://github.com/baileyrd/rusty_repo_wise/issues/56)

- **Added:** `primitive_obsession`, flagging a function/method whose
  declared parameters lean on bare primitives (`i32`/`bool`/`String` and
  language equivalents) instead of small domain-specific types — the
  classic "primitive obsession" smell. Unlike every other health marker
  so far, this one needs actual declared parameter *types*, a signal
  that only exists for statically-typed languages in this port's model.
- **`Symbol` gains `primitive_param_count: usize`**, populated at parse
  time.
- **New `repowise-parser::metrics::primitive_param_count`**, driven by
  two per-language closures: `param_type` extracts a parameter's
  declared type as source text, and `is_primitive_type` classifies it.
  - **Rust:** strips a leading `&`/`&mut`/lifetime reference prefix
    before classifying (so `&str`/`&'a String` count the same as their
    owned form), and treats `String`/`str` as primitives alongside the
    scalar keyword types (`i32`, `bool`, `usize`, etc.) — the smell
    targets overused strings/ints/bools, not Rust's `Copy` boundary.
  - **TypeScript:** reads the `type_annotation` node on
    `required_parameter`/`optional_parameter`; only
    `string`/`number`/`boolean` count (not `any`/`unknown`/`void`/etc.).
- **Scope:** implemented for **Rust and TypeScript only** for this first
  pass — the issue's own acceptance criteria required at least Rust,
  with TypeScript conditional on existing typed-parameter infra (which
  already existed via `repowise-parser::javascript`'s TypeScript
  grammar support). The other 14 parsed languages, including
  Python/JavaScript (which lack static type annotations in the common
  case and would need type inference this port doesn't have), get an
  empty parameter-type extraction and never trigger this marker.
  Extending to the remaining statically-typed languages (Java, Kotlin,
  Go, C, C++, C#, Scala, Swift, Dart) is a natural follow-up, not done
  here — same "scope to what the issue named, document the rest as a
  follow-up" pattern already used for LCOM4 (#51) and
  `complex_conditional` (#55).
- **New `FindingKind::PrimitiveObsession`** (penalty −0.3, same weight
  as `TooManyParameters`/`ComplexConditional`), flagged at
  `PRIMITIVE_OBSESSION_MIN_COUNT = 3` primitive-typed parameters.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-docs`/`repowise-git`/`repowise-health` that
  build `Symbol` directly.
- Workspace test count: 203 → 206.
- This closes out the last of the six health-marker parity-gap issues
  filed against `repowise-dev/repowise`'s documented health scorer
  (#51-#56).

---

## PR #127 — Add complex_conditional boolean-operator-count health marker
**2026-07-23** · [#127](https://github.com/baileyrd/rusty_repo_wise/pull/127) · closes [#55](https://github.com/baileyrd/rusty_repo_wise/issues/55)

- **Added:** `complex_conditional`, flagging a single `if`/`while`/etc.
  condition that chains 3+ boolean operators (`&&`/`||` in
  Rust/JS/TS, `and`/`or` in Python) — unlike `nested_complexity` (#53)
  and `bumpy_road` (#54), which are Symbol-level aggregate scalars, this
  marker's whole value is pointing at the *specific* condition, so it
  needed a different shape.
- **`Symbol` gains `complex_conditionals: Vec<ComplexConditionalRef>`**
  (each entry: `line`, `operator_count`), populated at parse time —
  mirroring how `field_accesses` already works as a per-symbol Vec
  collection, rather than one more `usize` scalar.
- **New `repowise-parser::metrics::complex_conditionals`**, driven by
  two per-language closures: `condition_of` extracts the `condition`
  sub-expression from an `if`/`while`/etc. node (language-specific field
  name), and `is_boolean_operator` — deliberately kept separate from
  each language's existing `is_decision` classifier — counts chained
  boolean operators within just that condition's own subtree, not the
  whole function body. Threshold: `COMPLEX_CONDITIONAL_MIN_OPERATORS = 3`.
- **Grammar verification:** before writing per-language `condition_of`
  logic, the vendored `node-types.json` for the exact tree-sitter grammar
  versions pinned in the workspace `Cargo.toml` (Rust 0.23.3, Python
  0.23.6, JavaScript 0.23.1) was inspected to confirm `condition` field
  names and boolean-operator node shapes, rather than guessing and
  iterating on failures.
- **Scope:** real extraction implemented for **Rust, Python, and
  TypeScript/JavaScript only** — the same three languages LCOM4 (#51)
  and near-duplicate detection (#52) needed new per-language grammar
  logic for. The other 13 parsed languages get an empty `Vec` via the
  same construction-site default already used for other per-symbol Vec
  fields.
- **New `FindingKind::ComplexConditional`** (penalty −0.3), emitting one
  `Finding` per flagged condition using that condition's own `line` —
  not the enclosing function's line — so downstream consumers
  (dashboard, wiki, `get_why`) can jump straight to the offending
  expression.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-docs`/`repowise-git`/`repowise-health` that
  build `Symbol` directly.
- Workspace test count: 200 → 203.

---

## PR #125 — Add bumpy_road nesting-bumps health marker
**2026-07-23** · [#125](https://github.com/baileyrd/rusty_repo_wise/pull/125) · closes [#54](https://github.com/baileyrd/rusty_repo_wise/issues/54)

- **Added:** `bumpy_road`, a structural-complexity marker complementing
  `nested_complexity` (#53): rather than the single deepest point
  reached, it counts how many *separate* nested-block regions occur
  within one function. Three separate two-level-deep blocks read worse
  than one two-level-deep block, even at the same max nesting depth — a
  case `max_nesting_depth` alone can't distinguish.
- **New `repowise-parser::metrics::bumpy_road_bumps`**, computed
  alongside `cyclomatic_complexity`/`max_nesting_depth` in one
  post-order AST pass. **Counting rule** (documented and tested): only
  *leaf* decision nodes count — a decision node with no further decision
  node nested inside it (before hitting a nested-function boundary) —
  reaching a nesting depth of at least 2. A linear chain (`if`
  containing `if` containing `if`) has exactly one leaf and counts as a
  single bump, not three, since it's one deep block rather than several
  scattered ones; three separate sibling `if`s each with one level of
  nesting inside have three leaves and count as three bumps.
- **`Symbol` gains `bumpy_road_bumps: usize`**, wired into all 16
  already-supported languages' function/method extraction — same
  scope/shape as `nested_complexity`: reuses each language's existing
  `is_decision`/`is_nested_function` closures, so no new per-language
  AST classification was needed.
- **New `FindingKind::BumpyRoad`** (threshold `>= 3` bumps, penalty
  −0.5, lighter than `NestedComplexity`'s −1.0 since it's a
  complementary signal on the same underlying data, not an independent
  problem worth double-weighting).
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-docs`/`repowise-git`/`repowise-health` that
  build `Symbol` directly.
- 3 new tests (a dedicated bumps-vs-depth test in both `rust.rs` and
  `python.rs` — two functions at identical max nesting depth, one with
  three scattered two-level blocks and one with a single two-level
  block, confirming `bumpy_road_bumps` tells them apart; a
  `repowise-health` test confirming a function at/above the bump
  threshold is flagged and one below it isn't), 200 tests passing
  workspace-wide (up from 197). Next up per the loop is issue #55
  (`complex_conditional` — boolean operator count per condition), the
  fifth of six filed health-marker issues.

## PR #123 — Add nested_complexity max-nesting-depth health marker
**2026-07-23** · [#123](https://github.com/baileyrd/rusty_repo_wise/pull/123) · closes [#53](https://github.com/baileyrd/rusty_repo_wise/issues/53)

- **Added:** `nested_complexity`, a structural-complexity marker
  measuring maximum control-flow nesting depth per function/method —
  complements cyclomatic complexity, which counts decision points flat:
  a function with 10 sequential ifs and one with the same 10 ifs nested
  inside each other score identically on cyclomatic complexity but read
  very differently, and only nesting depth tells them apart.
- **New `repowise-parser::metrics::max_nesting_depth`**, a recursive AST
  walk alongside the existing `cyclomatic_complexity` — reuses the exact
  same per-language `is_decision`/`is_nested_function` classification,
  just tracking how deep decision-classified nodes nest inside each
  other (incrementing depth only when descending into one) rather than
  counting them flat.
- **`Symbol` gains `max_nesting_depth: usize`**, wired into all 16
  already-supported languages' function/method extraction in this one
  PR. Unlike LCOM4 (#51), which needed genuinely new per-language
  field-access extraction and was deliberately scoped to 3 languages,
  this marker needed no new AST classification logic — every language's
  `is_decision` already existed for `cyclomatic_complexity` — so there
  was no scoping decision to make here.
- **New `FindingKind::NestedComplexity`** (threshold `> 4` levels,
  penalty −1.0, matching `HighComplexity`'s weight since both are cheap
  AST-derived structural signals of comparable severity).
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-docs`/`repowise-git`/`repowise-health` that
  build `Symbol` directly.
- 3 new tests (a dedicated nesting-depth-vs-complexity test in both
  `rust.rs` and `python.rs` — two functions with identical cyclomatic
  complexity, one sequential and one nested, confirming
  `max_nesting_depth` tells them apart; a `repowise-health` test
  confirming a function above the threshold is flagged and one exactly
  at the threshold isn't), 197 tests passing workspace-wide (up from
  194). Next up per the loop is issue #54 (`bumpy_road` — nesting
  "bumps" per method), the fourth of six filed health-marker issues.

## PR #121 — Add dry_violation near-duplicate code health marker
**2026-07-23** · [#121](https://github.com/baileyrd/rusty_repo_wise/pull/121) · closes [#52](https://github.com/baileyrd/rusty_repo_wise/issues/52)

- **Added:** `dry_violation`, a near-duplicate-code detector catching
  *partial* duplicates that the existing exact-body-hash `Duplicate
  code` marker misses entirely — a function that's mostly identical to
  another with a few renamed variables or a tweaked constant, where even
  one differing character breaks a hash match. New
  `repowise-health::near_duplicate` module, new
  `FindingKind::NearDuplicateCode` (penalty −0.3, lighter than
  `DuplicateCode`'s −0.5 since it's a heuristic overlap ratio rather
  than a byte-for-byte match).
- **Tokenized, not raw-character, Rabin-Karp windows.** Each candidate
  symbol's source is tokenized (identifier/number runs plus
  single-character punctuation) before windowing, rather than sliding a
  window over raw normalized characters — an identifier rename changes
  *length* (`total` -> `sum`), which would shift every subsequent
  character position and misalign every raw-character window from that
  point on even though the code is otherwise identical. A token-level
  window only invalidates the windows actually containing the renamed
  token. Verified this empirically against a realistic renamed-variable
  fixture before landing on a 3-token window and a 50% overlap
  threshold — an earlier 40-character/60%-overlap attempt scored a
  genuine near-duplicate pair at 0% overlap due to exactly this
  misalignment problem.
- **Rabin-Karp bucketing, not brute-force all-pairs comparison:** two
  symbols only become a "candidate pair" once they share at least one
  window hash; pairs with nothing in common are never compared at all.
  Candidate pairs are then scored by shared-window-count ÷ the smaller
  symbol's window count.
- **Explicitly excludes pairs already caught by `DuplicateCode`**
  (identical `body_hash`) so a pair is never reported under both finding
  kinds at once — the two answer different questions ("identical" vs
  "mostly the same").
- **Architectural note:** this is the first marker in `repowise-health`
  that isn't a pure function of already-computed `RepoIndex`/`RepoGraph`
  data — `Symbol` doesn't carry raw body text, so it re-reads each
  candidate symbol's file fresh from disk, the same tradeoff
  `repowise-mcp::get_symbol` and the ADR code-comment/inline-marker
  sources already make elsewhere in this workspace. A file moved or
  deleted since indexing degrades that file's contribution to empty
  rather than failing the whole scan.
- 5 new tests (a genuinely near-duplicate pair with a renamed
  accumulator variable and a tweaked constant is flagged; genuinely
  different functions aren't; a pair already caught by the exact-hash
  marker is excluded; symbols too short to have a `body_hash` are
  skipped; a file missing from disk degrades gracefully), 194 tests
  passing workspace-wide (up from 189). Next up per the loop is issue
  #53 (`nested_complexity` — max nesting depth), the third of six filed
  health-marker issues.

## PR #119 — Add LCOM4 low_cohesion health marker (Rust/Python/TS+JS)
**2026-07-23** · [#119](https://github.com/baileyrd/rusty_repo_wise/pull/119) · closes [#51](https://github.com/baileyrd/rusty_repo_wise/issues/51)

- **Added:** `low_cohesion` (LCOM4), a structural-complexity health
  marker documented as a known deferred item since PR #12 ("needs
  field-level access tracking per method"). `repowise-parser` now tracks
  per-method `self`/`this` field reads/writes — `field_expression`
  (Rust), `attribute` (Python), `member_expression` (TypeScript/
  JavaScript) — into a new `FieldAccessRef` record on `FileRecord`. A
  new `is_call_target` check per language excludes `self.method()`/
  `this.method()` call targets from the signal, so method names don't
  pollute the field-cohesion data.
- **Scope decision:** field-access extraction covers **Rust, Python, and
  TypeScript/JavaScript only** — the three languages issue #51's own
  acceptance criteria named explicitly, out of the 16 languages this
  port parses. Sized this up before implementing: `Symbol.parent`
  already tracks class/impl ownership, and the extraction pattern is a
  direct copy-adapt of the existing call-target extraction (same AST
  node kinds, same walker structure), which made this a single-PR-scale
  effort rather than a multi-day one — so all three named languages
  landed together here rather than splitting by language. The other 13
  languages have an empty `field_accesses` list per file and are
  silently skipped for this one marker (not enough data, not
  "cohesive"), not flagged either way.
- **New `repowise-health::lcom4` module:** per class, builds a graph
  where methods are nodes and an edge connects two methods sharing at
  least one field, then counts connected components via a small
  hand-rolled union-find (not a new graph-library dependency — per-class
  method counts are small enough that this is simpler than pulling in
  one). A class whose field-touching methods split into 2+ disjoint
  components is flagged as `FindingKind::LowCohesion` (penalty −1.0).
- **Methods with zero recorded field access are excluded from the graph
  entirely**, not counted as their own singleton component — otherwise
  almost any real-world class would trip this marker the moment it
  contains one delegator/pure-helper method that never touches a field
  directly. A class needs at least 2 field-touching methods before "do
  they share fields" is even a meaningful question.
- 8 new tests (3 parser field-access extraction — one per language,
  each confirming reads/writes are recorded and same-receiver method
  calls are not — and 5 `lcom4` tests: a genuinely low-cohesion class,
  a cohesive class, a class with an excluded zero-access delegator, a
  class below the tracked-method threshold, and a language with no
  field-access extraction skipped rather than flagged), 189 tests
  passing workspace-wide (up from 181). Next up per the loop is issue
  #52 (`dry_violation` — Rabin-Karp near-duplicate detection), the
  second of six filed health-marker issues.

## PR #117 — Expand commit-message decision-keyword list
**2026-07-23** · [#117](https://github.com/baileyrd/rusty_repo_wise/pull/117) · closes [#50](https://github.com/baileyrd/rusty_repo_wise/issues/50)

- **Widened `commits::DECISION_KEYWORDS`** from 7 entries (`decide`
  through `instead of`) to 19, toward the reference's documented
  "git archaeology" keyword set. `migrate`/`replace`/`deprecate`/`drop`/
  `rewrite`/`split`/`revert` are named explicitly in issue #50;
  `opt for`/`in favor of`/`settle on`/`consolidate`/`standardize on`
  round the list out to 19 from common decision-language vocabulary —
  the reference repo wasn't reachable from this session to confirm its
  exact remaining entries, so that last group is a documented best
  effort rather than a verified match.
- No logic change — `is_decision_message`'s case-insensitive substring
  match over the keyword list is unchanged; this is purely a data
  (const array) change plus tests.
- **This closes out all filed ADR-mining issues (#46-50)**:
  `repowise-adr::mine` now draws on six independent sources — ADR files,
  commit messages (this widened list), merged PR bodies, decision-like
  code comments, inline decision markers, and keep-a-changelog CHANGELOG
  sections.
- 2 new tests (all 12 newly-added keywords individually flagged as
  decision-like; an unrelated message stays unflagged), 181 tests
  passing workspace-wide (up from 179). Next up per the loop is issue
  #51, the first of six filed health-marker issues (#51-56): LCOM4
  (`low_cohesion`) structural-complexity scoring.

## PR #115 — Add CHANGELOG decision source to repowise-adr
**2026-07-23** · [#115](https://github.com/baileyrd/rusty_repo_wise/pull/115) · closes [#49](https://github.com/baileyrd/rusty_repo_wise/issues/49)

- **Added:** a sixth architectural-decision source — keep-a-changelog-
  style CHANGELOG sections. A new `DecisionSource::Changelog { file,
  section }` variant, and a new `repowise-adr::changelog` module that
  finds whichever of `CHANGELOG.md`/`HISTORY.md`/`NEWS.md`/`CHANGES.md`
  exists at the repo root first (checked in that priority order,
  case-insensitive, so the result is deterministic even if more than one
  happens to exist) and scans it for `### Changed`/`### Removed`/
  `### Deprecated`/`### Security` section headings — a heading-text
  match, not a full keep-a-changelog spec parser, per this issue's own
  acceptance criteria.
- **`### Added`/`### Fixed` are deliberately excluded** — purely
  additive or bug-fix entries aren't architectural decisions the way a
  change/removal/deprecation/security call generally is.
- **Linking treatment differs from the last three sources.** A
  changelog entry's `linked_files` goes through the same text-matching
  linker ADR files and commit messages already use in `mine()`, rather
  than the authoritative self-link PR/code-comment/inline-marker
  decisions get. A changelog entry isn't "about" the changelog file
  itself — it's prose describing a change made somewhere else in the
  codebase — unlike a PR's diff or the file a comment sits in, which
  genuinely are the thing the decision is about.
- **Pure filesystem/parsing, no new dependency** — this repo's own
  `RELEASE_NOTES.md` was a reasonable first fixture to think through per
  the issue's own note, but the tests use a proper keep-a-changelog-
  shaped fixture, since the source itself needs to support the standard
  convention generically, not just this repo's own format.
- `DecisionSource` gaining a variant is a breaking change for any
  exhaustive match over it, same as the three decision-source PRs before
  this one — updated `repowise-cli::cmd_decisions` and
  `repowise-mcp::get_why` accordingly, verified via a full workspace
  build.
- 5 new tests (each recognized section mined from a keep-a-changelog
  fixture, case-insensitive filename matching, falling back to
  `HISTORY.md` when no `CHANGELOG.md` exists, no changelog file at all
  degrades to empty, `### Added`/`### Fixed` correctly ignored), 179
  tests passing workspace-wide (up from 174). Next up per the loop is
  issue #50 — a small, low-risk enhancement to the *existing*
  commit-message source rather than a new one: widening
  `DECISION_KEYWORDS` toward the reference's fuller ~19-verb list.

## PR #113 — Add inline decision marker mining to repowise-adr
**2026-07-23** · [#113](https://github.com/baileyrd/rusty_repo_wise/pull/113) · closes [#48](https://github.com/baileyrd/rusty_repo_wise/issues/48)

- **Added:** a fifth architectural-decision source — inline decision
  markers. A new `DecisionSource::InlineMarker { file, line, marker }`
  variant, and a new `repowise-adr::inline_markers` module recognizing a
  small, explicit tag vocabulary (`WHY:`, `DECISION:`, `TRADEOFF:`,
  `ADR:`, `RATIONALE:`, `REJECTED:`) as a prefix inside any comment
  syntax (`#`, `//`, `/* */`), wherever it appears in a file — not tied
  to sitting above a symbol's declaration the way the code-comment
  source is. Much lower false-positive risk than that freeform source:
  this is an explicit opt-in convention, not a keyword guess, so every
  match is deliberate.
- **A plain text scan, not language-specific parsing** — `comment_lines`
  tracks `/* ... */` block state line-by-line across the whole file;
  `//`/`#` line comments are recognized only when they start a line
  (a trailing `code(); // WHY: ...` is out of scope for this simple
  scan, a documented limitation).
- **Deliberately doesn't reuse `code_comments::comment_block_above`** —
  evaluated it first (per issue #47's own note to check before
  duplicating logic) and decided against it. That helper answers "what's
  the comment block directly above *this specific symbol*"; inline
  markers need "every comment line in the file, wherever it sits" (a
  marker doesn't have to sit above a declaration at all). Reusing it
  would have meant calling it once per symbol and *still* needing a
  separate whole-file scan for markers not adjacent to any declaration —
  more complexity than scanning the file once directly. Reasoning
  recorded in the module's own doc comment.
- **Linked to the file the marker sits in** — the same "authoritative,
  not text-matched" treatment PR and code-comment decisions already get
  in `mine()`'s linking pass.
- **A noted (not a bug) overlap with the code-comment source**: a line
  like `# DECISION: adopt sled` will independently match both this
  source (the `DECISION:` tag) and the freeform code-comment heuristic
  if it happens to sit directly above a symbol (since "DECISION:"
  contains "decision"), producing two separate `DecisionRecord`s for the
  same line. Consistent with how every decision source in this crate is
  already independent and undeduplicated against the others — not
  something this PR changes.
- `DecisionSource` gaining a variant is a breaking change for any
  exhaustive match over it, same as the two decision-source PRs before
  this one — updated `repowise-cli::cmd_decisions` and
  `repowise-mcp::get_why` accordingly, verified via a full workspace
  build.
- 6 new tests (every marker tag recognized in `#` syntax, `//` syntax, a
  `/* */` block, correct file-linking, a plain comment with no tag
  ignored, a look-alike word like "ADRENALINE:" correctly not matched),
  174 tests passing workspace-wide (up from 168). Next up per the loop
  is issue #49, CHANGELOG-based decision mining.

## PR #111 — Add code-comment decision source to repowise-adr
**2026-07-23** · [#111](https://github.com/baileyrd/rusty_repo_wise/pull/111) · closes [#47](https://github.com/baileyrd/rusty_repo_wise/issues/47)

- **Added:** a fourth architectural-decision source — decision-like
  comments/docstrings sitting directly above an indexed symbol's
  declaration. A new `DecisionSource::CodeComment { file, line }`
  variant, and a new `repowise-adr::code_comments` module applying the
  same decision-keyword heuristic `commits.rs`/`pull_requests.rs`
  already use to whatever comment block sits immediately above each
  symbol's `start_line`. Pure filesystem/parsing — no new dependency,
  unlike the PR-body source before it.
- **`comment_block_above` handles two comment shapes**: a contiguous run
  of `//`- or `#`-prefixed lines, or a `/* ... */` block, walked upward
  from its closing `*/` to the matching opening `/*` so a multi-line
  JavaDoc/rustdoc-style comment is captured whole rather than just its
  last line.
- **Deliberately scoped to "immediately above, no blank-line gap"** — the
  common doc-comment convention across most languages this port parses.
  Python/JavaScript's alternative convention (a docstring as the
  function body's first statement) isn't handled; a documented gap, not
  a silent one, called out in the module doc comment and README.
- **Linked to the file the comment sits in directly** — the same
  "authoritative, not text-matched" treatment PR decisions already get
  in `mine()`'s linking pass, for the same reason: text-matching could
  only ever throw away information this source already knows for
  certain.
- **Groundwork left for issue #48** (inline decision markers — `# WHY:`,
  `# DECISION:`, etc.): `comment_block_above` is written as its own
  reusable unit specifically so that source can reuse the
  "find-the-comment-block-above-a-symbol" half of the work and add only
  its own marker-tag matching on top, rather than duplicating comment
  discovery. Issue #48 hadn't landed yet when this PR was written, so
  there was nothing to deduplicate against yet — checked per issue #47's
  own note about overlapping logic.
- `DecisionSource` gaining a variant is a breaking change for any
  exhaustive match over it, same as the PR-body PR before this one —
  updated `repowise-cli::cmd_decisions` and `repowise-mcp::get_why`
  accordingly, verified via a full workspace build.
- 4 new tests (a decision-like line comment, a decision-like block
  comment, a non-decision comment correctly ignored, a comment separated
  from its symbol by a blank line correctly not mined), 168 tests
  passing workspace-wide (up from 164). Next up per the loop is issue
  #48, inline decision markers.

## PR #109 — Add PR-body decision source to repowise-adr
**2026-07-23** · [#109](https://github.com/baileyrd/rusty_repo_wise/pull/109) · closes [#46](https://github.com/baileyrd/rusty_repo_wise/issues/46)

- **Added:** a third architectural-decision source — merged PR bodies,
  mined via the GitHub API. A new `DecisionSource::PullRequest { number,
  author }` variant, and a new `repowise-adr::pull_requests` module
  applying the same decision-keyword heuristic `commits.rs` already uses
  (`is_decision_message` is now `pub(crate)` and reused, not duplicated)
  to each merged PR's title/body. Unlike the other two sources, a PR
  decision links to the files that PR actually touched — reported
  directly by the GitHub API — rather than falling back to text-matching
  against the index.
- **Opt-in, not automatic.** This is the first network call
  `repowise-adr` (previously pure git/filesystem) has ever made, and it's
  deliberately conservative about making one at all: only attempted when
  a `REPOWISE_GITHUB_TOKEN` env var is set, `root` is a git repo with a
  GitHub-hosted `origin` remote, and the API call succeeds — any one of
  those failing degrades to an empty result, same "not required"
  tradeoff already used for `docs/adr/` and git history. A local
  codebase-analysis CLI making unsolicited outbound HTTP requests would
  be surprising, so this requires an explicit token rather than falling
  back to GitHub's unauthenticated (and much more rate-limited) API.
- **New `ureq` dependency** — a synchronous HTTP client, chosen
  deliberately over an async one (`reqwest`) specifically to avoid
  pulling `tokio` into what's otherwise a plain git/filesystem crate.
  `repowise-mcp` remains the only other `tokio` consumer in this
  workspace, for its stdio server.
- **A proxy-rewrite bug caught before it shipped:** the remote URL is
  read via `git config --get remote.origin.url`, not `git remote
  get-url origin` — the latter applies any configured
  `url.<base>.insteadOf` rewrite (this sandbox's own git config rewrites
  `github.com` URLs through a local proxy for its own purposes), which
  would have pointed the owner/repo parser at the wrong host entirely.
  Caught by a test asserting the exact remote URL round-trips
  unmodified, which failed against a real proxy rewrite in this very
  environment before the fix.
- `DecisionSource` gaining a variant is a breaking change for any
  exhaustive match over it — updated `repowise-cli::cmd_decisions` and
  `repowise-mcp::get_why` accordingly, verified via a full workspace
  build.
- 9 new tests (GitHub remote URL parsing across SSH/HTTPS/`ssh://`
  forms and rejection of non-GitHub remotes, decision-keyword mining
  linked to real PR file lists, the actual HTTP/JSON request/response
  path exercised against a hand-rolled local TCP fixture server rather
  than a live network call or a new mocking-crate dependency, and the
  four degrade-to-empty paths: no token, no remote, a non-GitHub remote,
  and `git_remote_url` itself), 164 tests passing workspace-wide (up
  from 155). Next up per the loop is issue #47, code-comment decision
  mining — pure filesystem/parsing work, no new dependency this time.

## PR #107 — Add get_dead_code MCP tool with confidence tiers
**2026-07-23** · [#107](https://github.com/baileyrd/rusty_repo_wise/pull/107) · closes [#45](https://github.com/baileyrd/rusty_repo_wise/issues/45)

- **Added:** an eighth MCP tool, `get_dead_code(min_confidence?, safe_only?, limit?)`,
  and a new `repowise_health::find_dead_code` behind it — a richer
  sibling to the existing `possibly-dead-code` health marker rather than
  a thin wrapper over it. Both start from the same base signal (zero
  resolved in-repo callers), but `find_dead_code` tiers each candidate
  `low`/`medium`/`high` by two cheap, fully-documented risk factors:
  1. **Ambiguous name** — another symbol elsewhere in the index shares
     this exact name. Call resolution prefers a same-file match and
     otherwise fans out to every same-named candidate, so a call meant
     for this symbol could have resolved to the other same-named one
     instead — the "zero callers" reading is less trustworthy.
  2. **Same-stem unresolved import elsewhere** — an import elsewhere in
     the repo failed to resolve, and its last path segment matches this
     symbol's file stem: plausibly a missed attempt to import this file.

  Zero risk factors → `high`; one → `medium`; both → `low`. Shell is
  exempt entirely, same as the existing marker and for the same reason
  (a shell function's real callers — the command line, another script,
  cron — are invisible to this port's call graph).
- **`RepoGraph` gains `unresolved_import_stems`** (a `HashSet<String>`,
  populated during `build()` right alongside the existing
  `unresolved_imports` counter) — the one piece of raw resolution data
  neither `RepoIndex` nor the existing `Overview` aggregate exposed,
  needed for risk factor 2. Purely additive; no existing field changed.
- **A dead end worth recording:** the first design also tracked
  `unresolved_call_names`, meant to flag "a call elsewhere shares this
  symbol's name but didn't resolve." Tracing through `repowise-graph`'s
  actual call-resolution logic showed that set could never contain a
  name matching any real indexed symbol — if a name exists anywhere in
  the index, resolution always finds at least one candidate for it, so
  the check could never fire. Dropped before it shipped as dead code in
  favor of the "ambiguous name" signal above, which needs no `RepoGraph`
  change at all (derivable straight from `RepoIndex`).
- `min_confidence` (`"low"`/`"medium"`/`"high"`, case-insensitive) filters
  to that tier and above; `safe_only` narrows to `high` only — the
  closest this gets to the reference's "safe to delete" designation,
  though the tool description explicitly says this is **not** a
  runtime-safety guarantee at any tier (reflection, dynamic dispatch, and
  entry points are all invisible to this port's static call graph, same
  caveat the existing marker already carries). `limit` (default 50) caps
  the returned list; `total_matching` in the response reports the count
  before truncation.
- 11 new tests (7 in `repowise-health` covering the tiering logic and
  sort order directly, 4 in `repowise-mcp` covering the tool's
  filtering/limiting/error-handling), 155 tests passing workspace-wide
  (up from 144). This closes out the last of the filed MCP-tool issues
  (#41-45). Next up per the loop is issue #46, PR-body decision mining —
  worth a heads-up before starting: it needs a GitHub API call to fetch
  merged PR bodies, a new kind of dependency for `repowise-adr` (currently
  pure git/filesystem), which the issue itself flags as worth calling out
  rather than adding silently.

## PR #105 — Add get_why MCP tool
**2026-07-23** · [#105](https://github.com/baileyrd/rusty_repo_wise/pull/105) · closes [#44](https://github.com/baileyrd/rusty_repo_wise/issues/44)

- **Added:** a seventh MCP tool, `get_why(targets?)`, returning
  architectural decisions mined via `repowise-adr::mine` whose body links
  to at least one of the given `targets`' files — same data as `repowise
  decisions --for-file`. With no targets (or an empty list), returns
  every mined decision.
- **A thin wrapper, no new mining logic.** `repowise-adr` already mines
  `docs/adr/*.md` and decision-like commit messages and links each to the
  files it mentions; `get_why` calls `mine()` fresh on every call (the
  same "no caching" rule every other tool follows) and just filters the
  result. Mirrors how `get_overview`/`search_codebase` already wrap
  existing library calls rather than reimplementing anything.
- **Targets can be file paths or symbol ids.** A target that exactly
  matches an indexed symbol's `id` (as returned by `search_codebase`/
  `get_context`, both extended with `id` in the `get_symbol` PR) resolves
  to that symbol's own file before filtering; anything else is treated
  as a file path, same resolution rules `get_context`/`get_risk` already
  use.
- `repowise-mcp` gains `repowise-adr` as a new dependency (previously
  `repowise-core`/`repowise-graph`/`repowise-health`/`repowise-git` only).
- 4 new tests (no targets returns every decision, filter by file target,
  filter by symbol target, an unmatched target returns nothing), 144
  tests passing workspace-wide (up from 140). Next up per the loop is
  issue #45, `get_dead_code` — a larger (L-sized) tool needing confidence
  tiering beyond this port's existing single-signal dead-code marker, so
  it wasn't folded into the smaller `get_symbol`/`get_why` additions.

## PR #103 — Add get_symbol MCP tool
**2026-07-23** · [#103](https://github.com/baileyrd/rusty_repo_wise/pull/103) · closes [#43](https://github.com/baileyrd/rusty_repo_wise/issues/43)

- **Added:** a sixth MCP tool, `get_symbol(symbol_id, context_lines?)`,
  returning one indexed symbol's raw source text. All the data needed
  (file, `start_line`/`end_line`) already lived in `RepoIndex` — this
  just slices the file's source at that span. `context_lines` (default
  `0`) pads the span by the same number of lines on each side, clamped to
  the file's real bounds rather than erroring on an out-of-range request.
- **`SymbolMatch` now includes each symbol's `id`.** Neither
  `search_codebase` nor `get_context` previously exposed a symbol's id,
  so there was no way for a caller to actually obtain one to pass to
  `get_symbol`. Both tools share the `SymbolMatch` output shape, so
  adding `id` there covers both call sites at once — purely additive, no
  existing field removed or renamed.
- **Reads the file fresh from disk on every call**, not from any content
  cached in the index — the same "don't trust the index for content,
  only for line metadata" tradeoff `repowise-docs`'s freshness tracking
  already makes. This means edits since the last `init`/`update` are
  reflected, at the cost of the returned span possibly being off if line
  numbers have since shifted.
- **Guards against a shrunk file.** `end_line` is clamped against the
  freshly re-read file's actual line count first; `start_line` is then
  clamped to never exceed that (already-clamped) `end_line`. Without the
  second clamp, a file that shrank since indexing could produce a
  `start_line > end_line` slice and panic.
- 3 new tests (own span by default, context-padding clamped to file
  bounds, unknown-id error), 140 tests passing workspace-wide (up from
  137). Next up per the loop is issue #44, `get_why` — a thin MCP wrapper
  over `repowise-adr`'s existing decision mining.

## PR #101 — Add get_change_risk MCP tool
**2026-07-23** · [#101](https://github.com/baileyrd/rusty_repo_wise/pull/101) · closes [#42](https://github.com/baileyrd/rusty_repo_wise/issues/42)

- **Added:** a fifth MCP tool, `get_change_risk(revspec?)`, computing a
  deterministic 0-10 diff-shape risk score for a single commit or a
  `base..head` range (defaulting to `HEAD`). A new `repowise-git::change_risk`
  function shells out to `git diff`/`git show --numstat --no-renames` and
  `git rev-list --count --author` to extract five metrics: lines added/
  deleted, files touched, subsystems touched (distinct top-level path
  components among the touched files), change concentration (Shannon
  entropy of each touched file's share of total lines changed, normalized
  by the maximum entropy for that file count so it's comparable across
  diffs of different sizes), and the head commit author's prior-commit
  count as an experience proxy. These combine via a fixed, documented
  weighting (0.25 lines, 0.20 each for files/subsystems/author-experience,
  0.15 concentration), each component saturating at a round, legible
  threshold rather than growing unbounded.
- **Deliberately not the reference's tool.** Per this issue's own scope
  note, the original repowise feeds the same kind of diff-shape metrics
  into a pre-trained L2-logistic-regression model. This port has no
  labeled defect corpus or model-training pipeline to reproduce that (see
  the category-A "ML-calibrated scoring" issue), so `get_change_risk`'s
  score is a simple, transparent heuristic instead — its tool description
  says so explicitly, so a caller can't mistake the number for a
  calibrated probability. The `--author` value passed to `git rev-list`
  is regex-escaped before use, since it's built from a git-reported email
  address that could otherwise contain regex metacharacters.
- Unlike `get_risk`, this tool never touches `RepoIndex`/`RepoGraph` at
  all — it's pure `git` plumbing, so it errors (rather than degrading to
  zero) when the indexed root isn't a git repository, since there's no
  diff to compute at all.
- 8 new tests (5 in `repowise-git`'s own `change_risk` module covering
  the metric extraction and scoring formula directly, 3 in
  `repowise-mcp` wiring/degradation), 137 tests passing workspace-wide
  (up from 129). Next up per the loop is issue #43, `get_symbol`.

## PR #99 — Add get_risk MCP tool
**2026-07-23** · [#99](https://github.com/baileyrd/rusty_repo_wise/pull/99) · closes [#41](https://github.com/baileyrd/rusty_repo_wise/issues/41)

- **Added:** a fourth MCP tool, `get_risk`, exposing `repowise-git`'s
  hotspot/churn/bug-fix-commit analytics alongside `repowise-health`'s
  findings for the same file — essentially `get_context` plus git
  history. Given a `file`, returns that file's hotspot score (churn ×
  total symbol complexity), raw churn, bug-fix-commit count, health
  score, and health findings. Given no `file`, returns the `top_n`
  (default 10) riskiest files repo-wide, ranked by hotspot score. Both
  shapes return the same `{ files: [...] }` structure (one entry or
  many) rather than a tagged union, keeping the tool's output and its
  tests simpler.
- **New dependency:** `repowise-mcp` now depends on `repowise-git`
  (previously only `repowise-core`/`repowise-graph`/`repowise-health`).
  Git analytics degrade to zero/empty via `GitAnalytics::collect(...).ok()`
  rather than erroring the whole call when the indexed root isn't a git
  repository — the same degrade-gracefully pattern `repowise-dashboard`
  already established, reused here for the first time in the MCP layer.
- 5 new tests (single-file risk with real git history, repo-wide top-N
  ranking, graceful degradation with no git repo, and the existing
  unindexed-file error path), 123 tests passing workspace-wide. Next up
  per the loop is issue #42, `get_change_risk` (deterministic scoring,
  not the reference's ML model).

## PR #97 — Add shell (sh/bash/zsh) language support
**2026-07-23** · [#97](https://github.com/baileyrd/rusty_repo_wise/pull/97) · closes [#40](https://github.com/baileyrd/rusty_repo_wise/issues/40)

- **Added:** a `repowise-parser` extractor for shell scripts, deliberately
  narrower in scope than every prior language per repowise's own
  documented tiering: functions only (shell has no classes/structs).
  `source`/`.` with a plain relative path resolves directly against the
  including script's own directory, same as C/C++/Ruby/Dart. The common
  `SCRIPT_DIR="$(dirname "$0")"` / `source "$SCRIPT_DIR/helper.sh"`
  idiom is explicitly recognized — since `$SCRIPT_DIR` is, by that
  idiom's own convention, the script's own directory, the remaining
  path suffix resolves the same way a plain relative `source` would.
  Any other expansion in the path (`$HOME`, `$(cmd)`, a differently-
  named variable) has no static value to resolve, so it's recorded but
  left unresolved. Every bareword command invocation is recorded as a
  call (indistinguishable, syntactically, from a call to an external
  program or builtin) — unresolvable ones are naturally filtered out by
  the existing name-index-based resolution.
- **`repowise-health`: shell is exempt from dead-code detection.** Per
  this issue's own acceptance criteria and repowise's documented
  shell-tier scope, shell functions are now unconditionally exempt from
  the possibly-dead-code marker (a new `skip_dead_code` parameter
  threaded through `check_function_markers`, keyed on
  `Language::Shell`) — a shell function is routinely invoked only from
  the command line, another script, or a cron job, none of which this
  port's call graph can see, making the signal too unreliable to report
  for this language. All other markers (long-function, high-complexity,
  too-many-params, duplicate-code) still apply to shell the same as
  everywhere else — confirmed both by a dedicated unit test and live
  through the CLI against a hand-built fixture with an intentionally
  uncalled function.
- 5 new `repowise-parser` unit tests, 1 new `repowise-graph` end-to-end
  test proving the `SCRIPT_DIR` idiom resolves, and 1 new
  `repowise-health` test proving the dead-code exemption; 118 tests
  passing workspace-wide. Thirteenth language merged out of this
  session's `parity-loop` gap-analysis pass (after TypeScript/JavaScript
  in #26, Java in #75, Kotlin in #77, Go in #79, C++ in #81, C# in #83,
  Scala in #85, Ruby in #87, C in #89, Swift in #91, PHP in #93, and
  Dart in #95) — this was the last of the filed B1 language-support
  issues; next up per the loop is whichever non-language `parity-gap`
  issue is oldest and unblocked (MCP tools, ADR sources, health markers,
  dashboard, or git analytics).

## PR #95 — Add Dart language support
**2026-07-23** · [#95](https://github.com/baileyrd/rusty_repo_wise/pull/95) · closes [#39](https://github.com/baileyrd/rusty_repo_wise/issues/39)

- **Added:** a `repowise-parser` extractor for Dart — classes/mixins map
  to `Class`/`Mixin` (reusing the `SymbolKind::Mixin` added for PHP —
  Dart's own `mixin` keyword is the same genuine-mixin concept),
  methods/functions nest via a `class_stack` the same way
  Java/Kotlin/Scala/PHP do. A method's `signature` field wraps a
  `method_signature`, itself wrapping the actual `function_signature`
  (name/parameters/return-type); bodiless abstract/interface method
  signatures use a shallower `declaration` node wrapping
  `function_signature` directly — both handled, recorded as symbols
  with 0 complexity for the bodiless case, same treatment as
  Java/Kotlin/Scala/PHP's bodiless methods.
- Relative `import 'local.dart'` resolves directly against the
  filesystem at parse time (mirroring TS/JS/C/C++/Ruby); `import
  'package:x/y.dart'` (a pub package) has no package registry here to
  resolve against, left unresolved by design, same tradeoff as bare npm
  specifiers.
- **Notable: bumped the shared `tree-sitter` core (0.24 → 0.25).**
  `tree-sitter-dart`'s only two published crates.io versions (`0.1.0`,
  `0.2.0`) both target grammar ABI 15, which `tree-sitter` 0.24's core
  doesn't support (max ABI 14) — unlike every previous ABI mismatch
  this session (C#, C, Swift, PHP), there was no older, ABI-14-compatible
  `tree-sitter-dart` release to pin instead. `tree-sitter` 0.25 widens
  its supported range to include ABI 15 while staying backward-compatible
  with the already-pinned older-ABI grammars (`tree-sitter-c-sharp`
  0.21, `tree-sitter-c` 0.21, `tree-sitter-swift` 0.6, `tree-sitter-php`
  0.23) — verified explicitly by bumping just the core version and
  re-running the full existing 106-test suite (all 11 other languages)
  before writing any Dart-specific code, confirming zero regressions
  from the core bump alone.
- 5 new tests (class/mixin/method extraction, relative/`package:`
  import handling, member/bare/constructor call tracking, cyclomatic
  complexity, duplicate-body hashing) plus a `repowise-graph` end-to-end
  test proving relative imports resolve while `package:` imports stay
  unresolved; 111 tests passing workspace-wide. Twelfth language merged
  out of this session's `parity-loop` gap-analysis pass (after
  TypeScript/JavaScript in #26, Java in #75, Kotlin in #77, Go in #79,
  C++ in #81, C# in #83, Scala in #85, Ruby in #87, C in #89, Swift in
  #91, and PHP in #93) — next up per the loop is Shell (#40).

## PR #93 — Add PHP language support
**2026-07-23** · [#93](https://github.com/baileyrd/rusty_repo_wise/pull/93) · closes [#38](https://github.com/baileyrd/rusty_repo_wise/issues/38)

- **Added:** a `repowise-parser` extractor for PHP — classes/interfaces/
  traits map to `Class`/`Trait`/`Mixin`, methods/functions nest via a
  `class_stack` the same way Java/Kotlin/Scala do.
- **New `SymbolKind::Mixin` variant:** PHP's own acceptance criteria
  list interfaces and traits as distinct concepts (a contract vs. a
  mixin of concrete implementations), and this port's existing `Trait`
  kind is already used consistently across languages for the
  interface-like concept, so conflating PHP's actual `trait` keyword
  into it would be more confusing than adding one narrowly-scoped
  variant. Blast radius was minimal: only one exhaustive `match` over
  `SymbolKind` existed (`label()`).
- **Two import mechanisms, both implemented:** `require`/`require_once`/
  `include`/`include_once` (four distinct grammar nodes, all wrapping a
  single expression) with a plain string literal argument resolve
  directly against the filesystem, same as C/C++/Ruby — concatenated
  forms like `require __DIR__ . "/other.php"` are recorded with no path
  at all, rather than guessed. `use Namespace\Class;` resolves via a new
  `php_namespace_path` heuristic (folder-mirrors-namespace, same
  convention as C#'s), reusing the existing `resolve_import` machinery
  with `sep = "\\"` — not aware of Composer's real `composer.json`
  autoload mapping.
- **Notable grammar quirk, caught by its own test:** PHP's `elseif`
  parses as a distinct `else_if_clause` node, not a nested `if_statement`
  — missing from `is_decision`'s initial pass caused the
  cyclomatic-complexity test to fail (expected 6, got 5) before it
  shipped.
- **Dependency note:** pins `tree-sitter-php = "0.23"` rather than the
  newer 0.24.x release — 0.24.2's grammar targets ABI 15, incompatible
  with this workspace's tree-sitter 0.24 core (ABI 13–14 only). 0.23.11
  is ABI-compatible, the same fix already applied to
  `tree-sitter-c-sharp`/`tree-sitter-c`/`tree-sitter-swift`.
- 6 new tests (class/interface/trait/method extraction, `use`-statement
  handling, `require_once`-vs-concatenated-include resolution,
  object-creation calls, cyclomatic complexity, duplicate-body hashing)
  plus a `repowise-graph` end-to-end test proving both import
  mechanisms resolve; 106 tests passing workspace-wide. Eleventh
  language merged out of this session's `parity-loop` gap-analysis pass
  (after TypeScript/JavaScript in #26, Java in #75, Kotlin in #77, Go in
  #79, C++ in #81, C# in #83, Scala in #85, Ruby in #87, C in #89, and
  Swift in #91) — next up per the loop is Dart (#39).

## PR #91 — Add Swift language support
**2026-07-23** · [#91](https://github.com/baileyrd/rusty_repo_wise/pull/91) · closes [#37](https://github.com/baileyrd/rusty_repo_wise/issues/37)

- **Added:** a `repowise-parser` extractor for Swift — classes/structs/
  enums/actors (all share one `class_declaration` grammar node,
  distinguished by its `declaration_kind` field) map to
  `Class`/`Struct`/`Enum`/`Class`; protocols map to `Trait`. Extensions
  re-open an existing type rather than declaring a new one, so they
  don't get their own symbol, but their name is still pushed onto the
  `class_stack` so extension methods are correctly attributed to the
  extended type. Protocol method requirements have no body at all (a
  distinct `protocol_function_declaration` node, not
  `function_declaration` with an absent body) — recorded as symbols
  with 0 complexity, same treatment as Java/Kotlin/Scala's bodiless
  methods.
- **Import resolution, by design:** Swift's `import` is module-level
  (`import Foundation`), not file-level — there's no per-file
  relative-import syntax and a module name has no file mapping without
  a full build graph. Imports are recorded (for visibility/stats) but
  always left unresolved by design, asserted directly by this PR's own
  graph-layer test rather than treated as a "resolves" case that
  happens to fail.
- **Notable grammar quirk:** unlike every other language done so far,
  Swift's `function_declaration` has no wrapping parameters-list node
  at all — `parameter` nodes are direct children of the function
  declaration itself, interspersed with its name/return-type/body.
  `param_count` is counted directly rather than via the shared
  `metrics::count_params` helper, which assumes a dedicated list node
  (using that helper here would have silently counted every child, not
  just parameters).
- **Dependency note:** pins `tree-sitter-swift = "0.6"` rather than the
  newer 0.7.x release — 0.7.3's grammar targets ABI 15, incompatible
  with this workspace's tree-sitter 0.24 core (ABI 13–14 only). 0.6.0
  is ABI-compatible, the same fix already applied to
  `tree-sitter-c-sharp`/`tree-sitter-c`.
- 6 new tests (class/struct/protocol/method extraction, extension-
  attribution-without-duplicate-symbol, module-import-stays-unresolved,
  bare/member call tracking, cyclomatic complexity, duplicate-body
  hashing) plus a `repowise-graph` end-to-end test proving module
  imports correctly stay unresolved; 100 tests passing workspace-wide.
  Tenth language merged out of this session's `parity-loop`
  gap-analysis pass (after TypeScript/JavaScript in #26, Java in #75,
  Kotlin in #77, Go in #79, C++ in #81, C# in #83, Scala in #85, Ruby
  in #87, and C in #89) — next up per the loop is PHP (#38).

## PR #89 — Add C language support
**2026-07-23** · [#89](https://github.com/baileyrd/rusty_repo_wise/pull/89) · closes [#36](https://github.com/baileyrd/rusty_repo_wise/issues/36)

- **Added:** a `repowise-parser` extractor for C — functions and structs
  (`SymbolKind::Function`/`Struct`). Simpler than C++'s: plain C has no
  member functions at all, so there's no `class_stack` — struct fields
  and function bodies never nest into each other. Quote-form
  `#include "local.h"` is resolved directly against the filesystem at
  parse time (mirroring C++'s own `resolve_include`); angle-form
  `#include <system>` stays unresolved by design.
- **Design decision, left open by #32:** the C/C++ `.h` ambiguity.
  `.h` stays unmapped to either language (`Language::Other`) — the same
  call already made for C++'s own extension set — rather than guessing
  via syntax-sniffing. This has a **more significant practical
  consequence for C than it did for C++**: C++ has alternate,
  unambiguous header extensions (`.hpp`/`.hh`/`.hxx`) commonly used in
  practice, but C conventionally uses `.h` for nearly all its headers
  with no alternate in common use — so a conventional
  `#include "foo.h"` split resolves against the filesystem fine at parse
  time, but never becomes a real graph edge, since the header itself is
  never indexed as a graph node. Demonstrated directly by this PR's own
  graph resolution test (asserted, not just described in prose).
- **Dependency note:** pins `tree-sitter-c = "0.21"` rather than the
  newer 0.24.x release — 0.24.2's grammar targets ABI 15, incompatible
  with this workspace's tree-sitter 0.24 core (ABI 13–14 only). 0.21
  predates the `LanguageFn` API and is ABI-compatible, the same fix
  already applied to `tree-sitter-c-sharp`.
- 5 new tests (struct/function extraction, quote/angle include
  handling, field/bare call tracking, cyclomatic complexity,
  duplicate-body hashing) plus a `repowise-graph` end-to-end test
  proving quote-form includes of recognized extensions resolve while
  conventional `.h` headers stay unresolved; 94 tests passing
  workspace-wide. Ninth language merged out of this session's
  `parity-loop` gap-analysis pass (after TypeScript/JavaScript in #26,
  Java in #75, Kotlin in #77, Go in #79, C++ in #81, C# in #83, Scala in
  #85, and Ruby in #87) — next up per the loop is Swift (#37).

## PR #87 — Add Ruby language support
**2026-07-23** · [#87](https://github.com/baileyrd/rusty_repo_wise/pull/87) · closes [#35](https://github.com/baileyrd/rusty_repo_wise/issues/35)

- **Added:** a `repowise-parser` extractor for Ruby — classes and
  modules (mapped to `Class`/`Module`), plus `def` methods (both
  instance and `def self.`-style class methods), nested via a
  `class_stack` the same way Java/Kotlin/Scala do. `require_relative` is
  resolved directly against the filesystem at parse time (mirroring
  TS/JS's relative-import resolution and C++'s quote-form `#include`),
  trying the exact path then appending a `.rb` extension; plain
  `require` is gem-based (`$LOAD_PATH`) with no static equivalent to
  resolve against, so it's recorded but left unresolved by design.
  `receiver.new` calls are recorded as a call to the receiver class
  itself (Ruby's equivalent of `new Type()`).
- **Notable grammar quirk, caught by its own test:** `tree-sitter-ruby`
  names several rules after their own bare keyword (`if`, `elsif`,
  `while`, `until`, `for`, `rescue`, `when`) and *also* keeps that
  keyword as an anonymous child token of the identical kind string —
  double-counting cyclomatic complexity until an `is_named()` guard was
  added to `is_decision`.
- **Known limitation, stated plainly:** bare parenless/argless method
  calls (`helper` with no receiver, parens, or args) aren't
  distinguishable from local variable references by the grammar itself,
  so they aren't recorded as calls — callers should use explicit parens
  (`helper()`) for a call to be tracked.
- 5 new tests (class/module/method extraction, `require_relative`/
  `require` handling, constructor-call tracking, cyclomatic complexity,
  duplicate-body hashing) plus a `repowise-graph` end-to-end test
  proving `require_relative` resolves while plain `require` stays
  unresolved; 89 tests passing workspace-wide. Eighth language merged
  out of this session's `parity-loop` gap-analysis pass (after
  TypeScript/JavaScript in #26, Java in #75, Kotlin in #77, Go in #79,
  C++ in #81, C# in #83, and Scala in #85) — next up per the loop is C
  (#36).

## PR #85 — Add Scala language support
**2026-07-23** · [#85](https://github.com/baileyrd/rusty_repo_wise/pull/85) · closes [#34](https://github.com/baileyrd/rusty_repo_wise/issues/34)

- **Added:** a `repowise-parser` extractor for Scala — classes, objects,
  and traits (mapped to `Class`/`Class`/`Trait`), plus `def` methods.
  Like Java/Kotlin (and unlike Go/C++), Scala methods are always
  declared directly inside their type's `template_body`, so scoping
  uses the same `class_stack` push/pop pattern. Bodiless `def`
  signatures (abstract methods in traits) parse as a distinct
  `function_declaration` node rather than `function_definition` with an
  absent body — both are handled and recorded as symbols with 0
  complexity, same treatment as Java/Kotlin's bodiless methods. `import`
  declarations are extracted (plain and wildcard `_` forms); call and
  `new`-style instance expressions are tracked as calls.
- **Known limitation, stated plainly:** grouped selector imports
  (`import foo.{Bar, Baz}`) resolve to the enclosing package (`foo`)
  rather than being expanded into one entry per selector — an accepted
  simplification, same tradeoff already made for other languages'
  wildcard imports. Curried multi-parameter-list `def`s
  (`def f(a: Int)(b: Int)`) only have their first parameter list
  counted toward `param_count`.
- **Dependency note:** `tree-sitter-scala = "0.23"` turned out to be
  ABI-compatible with this workspace's tree-sitter 0.24 core without
  any downgrade — unlike `tree-sitter-c-sharp`, which needed pinning to
  0.21 (see the #83 entry below).
- Reuses the shared `jvm_module_path` convention from Java/Kotlin for
  import resolution, extended with `src/main/scala`/`src/test/scala` as
  recognized sbt source roots — a mixed Java/Kotlin/Scala project
  resolves imports across all three.
- 6 new tests (class/trait/object/method extraction, plain/wildcard
  imports, object-creation calls, cyclomatic complexity, duplicate-body
  hashing, trait-method-signature handling) plus a `repowise-graph`
  end-to-end test proving sbt-layout package resolution; 84 tests
  passing workspace-wide. Seventh language merged out of this session's
  `parity-loop` gap-analysis pass (after TypeScript/JavaScript in #26,
  Java in #75, Kotlin in #77, Go in #79, C++ in #81, and C# in #83) —
  next up per the loop is Ruby (#35).

## PR #83 — Add C# language support
**2026-07-23** · [#83](https://github.com/baileyrd/rusty_repo_wise/pull/83) · closes [#33](https://github.com/baileyrd/rusty_repo_wise/issues/33)

- **Added:** a `repowise-parser` extractor for C# — classes, structs,
  interfaces, methods, and constructors. Unlike Go/C++, C# methods are
  always declared directly inside their type's body, so scoping uses
  the same `class_stack` push/pop pattern already established for
  Java/Kotlin. `using` directives are extracted as imports
  (plain/dotted/aliased forms handled, `using static` skipped);
  invocation and object-creation expressions are tracked as calls.
- **Dependency note:** pins `tree-sitter-c-sharp = "0.21"` rather than a
  newer 0.23.x release — 0.23.5's grammar targets ABI 15, incompatible
  with this workspace's tree-sitter 0.24 core (ABI 13–14 only). 0.21
  predates the `LanguageFn` API and is ABI-compatible, the same
  workaround pattern used transiently for Rust/Python early in this
  project.
- **Known limitation, stated plainly:** namespace resolution
  (`csharp_namespace_path`) is a folder-mirrors-namespace heuristic —
  nothing in C#/.NET enforces that convention the way Maven/Gradle or
  `go.mod` do for Java/Kotlin/Go, so a project that doesn't follow it
  won't resolve correctly. Like Go, it's keyed by directory rather than
  per-file, so multiple files sharing one namespace resolve to
  whichever was indexed last.
- 6 new tests (class/interface/method extraction, using-directive
  forms, object-creation calls, cyclomatic complexity, duplicate-body
  hashing, interface-signature-vs-real-method) plus a `repowise-graph`
  end-to-end test proving folder-based namespace resolution; 78 tests
  passing workspace-wide. Sixth language merged out of this session's
  `parity-loop` gap-analysis pass (after TypeScript/JavaScript in #26,
  Java in #75, Kotlin in #77, Go in #79, and C++ in #81) — next up per
  the loop is Scala (#34).

## PR #81 — Add C++ language support
**2026-07-23** · [#81](https://github.com/baileyrd/rusty_repo_wise/pull/81) · closes [#32](https://github.com/baileyrd/rusty_repo_wise/issues/32)

- **Added:** a `repowise-parser` extractor for C++ — classes, structs,
  functions, and methods. Like Go, out-of-class method definitions
  (`Ret Widget::area() {...}`) get their parent read directly from the
  qualified name's scope; unlike Go, in-class method *prototypes*
  (`int area();` inside the class body, no bodies) are also tracked via
  a `class_stack`, recorded as separate `Method` symbols — the same
  dual-symbol pattern already established for Java/Kotlin/Go interface
  signatures. Quote-form `#include "local.h"` is resolved directly
  against the filesystem (mirroring TS/JS's relative-import resolution);
  angle-form `#include <system>` has no include-path search list and
  stays unresolved by design.
- **Known limitation, stated plainly:** `.h` is deliberately left
  unmapped to any language (`Language::Other`) — it's ambiguous between
  C and C++, and this issue is C++-only (plain C is tracked separately
  as issue #36). Only unambiguous C++ extensions (`.cpp`/`.cc`/`.cxx`/
  `.hpp`/`.hh`/`.hxx`) are recognized for now.
- 5 new tests (class/prototype/out-of-class-definition extraction,
  quote/angle include handling, member/bare/qualified call tracking,
  cyclomatic complexity, duplicate-body hashing) plus a `repowise-graph`
  end-to-end test proving quote-includes resolve while angle-includes
  stay unresolved; 71 tests passing workspace-wide. Fifth language
  merged out of this session's `parity-loop` gap-analysis pass (after
  TypeScript/JavaScript in #26, Java in #75, Kotlin in #77, and Go in
  #79) — next up per the loop is whichever `parity-gap` issue is oldest
  and unblocked (C#, per the filing order).

## PR #79 — Add Go language support
**2026-07-23** · [#79](https://github.com/baileyrd/rusty_repo_wise/pull/79) · closes [#31](https://github.com/baileyrd/rusty_repo_wise/issues/31)

- **Added:** a `repowise-parser` extractor for Go — structs, interfaces
  (mapped to `Trait`), functions, and methods. Go has no nested class
  scoping (methods are top-level declarations carrying a receiver
  clause, never nested inside the struct itself), so unlike every other
  language done so far, a method's `parent` is read directly from its
  receiver's type name rather than tracked via a scope stack. Import
  paths are resolved via a new `go_module_path` convention anchored on
  the nearest `go.mod`'s `module` declaration, mirroring Rust's
  `Cargo.toml`-anchoring.
- **Known limitation, stated plainly:** Go packages are directories
  (every file in one shares an import path), but the module-path index
  is one-file-per-path — a multi-file package only keeps the
  last-processed file as its resolved import target. Import edges still
  land in the right package, just not necessarily the exact file a
  symbol is defined in.
- 6 new tests (struct/interface/method extraction with receiver-based
  parent resolution, plain/aliased imports, selector/bare call tracking,
  cyclomatic complexity, duplicate-body hashing, interface-method-
  signature handling) plus a `repowise-graph` end-to-end test proving
  cross-package resolution via a real `go.mod`; 65 tests passing
  workspace-wide. Fourth language merged out of this session's
  `parity-loop` gap-analysis pass (after TypeScript/JavaScript in #26,
  Java in #75, and Kotlin in #77) — next up per the loop is whichever
  `parity-gap` issue is oldest and unblocked (C++, per the filing order).

## PR #77 — Add Kotlin language support
**2026-07-23** · [#77](https://github.com/baileyrd/rusty_repo_wise/pull/77) · closes [#30](https://github.com/baileyrd/rusty_repo_wise/issues/30)

- **Added:** a `repowise-parser` extractor for Kotlin — classes,
  interfaces (mapped to `Trait`), objects, and functions/methods.
  `repowise-graph`'s Java-only `java_module_path` was generalized to
  `jvm_module_path`, now recognizing both `src/main/java`/`src/test/java`
  and `src/main/kotlin`/`src/test/kotlin` source roots with both
  languages sharing one module-path index, so a mixed Java/Kotlin project
  resolves imports across both. Kotlin has no `new` keyword, so class
  instantiation (`Widget()`) is already covered by ordinary
  call-expression handling — no separate node-kind handler needed, unlike
  Java/TS/JS.
- **Known limitation, stated plainly:** secondary constructors aren't
  extracted as symbols (only the primary constructor's parameters,
  captured implicitly as part of the class symbol's span) — a narrower
  scope than Java's explicit constructor-declaration handling, accepted
  to keep this PR's scope reasonable.
- 6 new tests (class/interface/object/method extraction, plain/aliased/
  wildcard imports, bare-invocation-as-class-call tracking, cyclomatic
  complexity, duplicate-body hashing, interface-method-signature
  handling) plus a `repowise-graph` end-to-end test proving cross-language
  resolution (a Kotlin file importing a Java class in the same project);
  59 tests passing workspace-wide. Third language merged out of this
  session's `parity-loop` gap-analysis pass (after TypeScript/JavaScript
  in #26 and Java in #75) — next up per the loop is whichever
  `parity-gap` issue is oldest and unblocked (Go, per the filing order).

## PR #75 — Add Java language support
**2026-07-23** · [#75](https://github.com/baileyrd/rusty_repo_wise/pull/75) · closes [#29](https://github.com/baileyrd/rusty_repo_wise/issues/29)

- **Added:** a `repowise-parser` extractor for Java — classes, interfaces
  (mapped to `Trait`), enums, records, methods, and constructors (recorded
  as methods). Interface method signatures with no body are still
  recorded as symbols (0 complexity), same treatment as Rust's
  trait-method signatures. `import`/`import static`/wildcard imports are
  resolved via a new `java_module_path` convention anchored on the
  conventional Maven/Gradle `src/main/java`/`src/test/java` source root
  when present (falling back to repo-root-relative otherwise, same
  heuristic tradeoff as Python's dotted-path resolution). `new Type(...)`
  is recorded as a call to the constructed class, matching TS/JS's
  `new_expression` treatment, so instantiated classes don't read as dead
  code.
- **Known limitation, stated plainly:** no classpath/JAR-dependency
  resolution — bare (non-source-tree) references are left unresolved,
  same tradeoff already made for npm packages and Cargo dependencies. A
  nonstandard source layout (not `src/main/java`-anchored) falls back to
  a repo-root-relative package path, which may not match the file's real
  package declaration.
- 6 new tests (class/interface/method/constructor extraction, plain/
  static/wildcard imports, `new`-expression call tracking, cyclomatic
  complexity, duplicate-body hashing, interface-method-signature
  handling) plus a `repowise-graph` end-to-end Maven-layout resolution
  test; 52 tests passing workspace-wide. Second language merged out of
  this session's `parity-loop` gap-analysis pass (after TypeScript/
  JavaScript in #26, and hotspot scoring in #73) — next up per the loop
  is whichever `parity-gap` issue is oldest and unblocked.

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
