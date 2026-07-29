# Release Notes

Notable changes to this repo, newest first. No tagged releases yet, so entries
are keyed by PR — or by commit, for the two early changes that predate this
repo routing work through PRs and for one later change that bypassed it.

---

## PR #271 — Docs: CI and branch protection
**2026-07-29**

- **Fixed a dangling reference.** `.github/workflows/ci-rust.yml` pointed at
  `references/ci-and-branch-protection.md`, a path that has never existed in
  this repo. Added the document at `docs/ci-and-branch-protection.md` and
  repointed the comment, rather than deleting the pointer — the thing it
  promised to explain is real and undocumented.
- **Documents the failure mode that actually bit us.** CI's steps are
  sequential, so a Format failure means Clippy, Test and the WASM steps never
  ran. A red run showing only a fmt violation is not evidence the tests pass.
  Commit `894d0a6` left `main` red on exactly that for two days.
- **Names `check` as the status check context** in both the doc and the
  workflow comment, since renaming the job would silently detach any protection
  rule matching on it.
- **Records that protection is not applied.** `main` currently reports
  `protected: false`; applying it needs admin credentials no automation here
  has. The doc gives the `gh api` call, explains each field, and points at the
  command rather than at itself as the source of truth — including that merge
  method is a repository setting, not part of protection.
- Linked from CONTRIBUTING.md, whose "no direct pushes" and "CI must be green"
  rules are the ones this gate would enforce.

---

## PR #263 — UI: symbol and decision detail views
**2026-07-28** · closes [#263](https://github.com/baileyrd/rusty_repo_wise/issues/263)

- **Added:** `GET /api/symbol`, `GET /api/decision`, and two detail views,
  deep-linkable at `#/symbols?id=<file>@<line>` and `#/decisions?id=<id>`.
  Completes the UI parity round (7 of 7).
- **Sequenced after #259 deliberately** — detail pages whose whole purpose is
  linking someone to one symbol or decision are worth little without deep
  links, and doing routing first meant not building the plumbing twice.
- **Detail endpoints rather than fattened list endpoints.** `/api/symbols`
  carries no complexity or call graph and `/api/decisions` carries only a
  linked-file *count*; enriching the lists would make every row pay for detail
  nobody asked for.
- **An empty callee list never reads as "calls nothing."** The symbol endpoint
  counts calls it couldn't resolve to an indexed symbol and reports them
  separately — on `blame_file` that's 2 resolved against **20 unresolved**.
  "No resolved callers" likewise says outright that heuristic resolution makes
  it not proof of disuse.
- **Supersession is shown in both directions.** `superseded_by` already
  existed; `supersedes` is derived by scanning the set, since a record only
  stores the forward link. A superseded decision leads with a loud marker and
  a link to its replacement — showing one silently would read as current
  guidance, the one way this view could actively mislead.
- **Unknown ids render not-found views**, not errors — a stale deep link should
  say it's stale.
- The hash parser gained a generic `id=` parameter alongside `file=`, and the
  selection-sync effect skips rewriting while a detail view is addressed, which
  would otherwise drop the id and bounce back to the index.
- 6 new tests (3 server, 3 routing; 20 total in the WASM crate).

---

## PR #259 — UI: one view per section, addressable by URL
**2026-07-28** · closes [#259](https://github.com/baileyrd/rusty_repo_wise/issues/259)

- **Split the single stacked page into one route per section.** All 19 sections
  previously rendered together on one scrolling page; each is now its own view
  at `#/<slug>`, with a nav and only the current view mounted.
- **Reload restores the view**, and the selected file rides along as
  `?file=<path>`, so a drill-down survives a refresh and can be shared.
- **Hash routing, not path routing.** `serve-dashboard` serves static files, so
  `/health` would 404 on reload without a server-side catch-all rewrite. A hash
  needs no server change, and present mode already used one — so this follows
  the existing convention instead of adding a second. **No new dependency**: no
  router crate, and a six-character percent-encoder rather than a URL-encoding
  crate.
- **`ROUTES` is the single source of truth** for the nav, the parser and the
  formatter, so a view can't be added to one and forgotten in another. A test
  round-trips every entry, which is what makes a dead nav link impossible.
- **Unknown addresses render a not-found state**, not a silent redirect to
  Overview — a stale bookmark should say so.
- **Present mode keeps working, and its exit is fixed as a side effect.**
  `#present/<n>` is treated as an overlay rather than a view, so it never
  clobbers the underlying route; exiting used to blank the hash and now
  restores the view you were on.
- 7 new routing tests (17 total in the WASM crate), including paths containing
  `#`, `?`, `&`, `%`, `+` and spaces — each would otherwise split the hash and
  lose the tail.

---

## PR #262 — UI: activity view (punch card + weekly trend)
**2026-07-28** · closes [#262](https://github.com/baileyrd/rusty_repo_wise/issues/262)

- **Added:** `repowise_git::commit_activity`, `GET /api/stats`, and a
  `StatsSection` drawing a day×hour punch card and a weekly trend as inline
  SVG. Fifth issue of the UI parity round.
- **No date dependency.** Day-of-week and hour come from integer arithmetic on
  epoch seconds (1970-01-01 was a Thursday, hence the `+4` shift to make day 0
  Sunday) rather than pulling in `chrono` for two divisions.
- **UTC, stated explicitly.** Git stores an author timezone offset this port
  doesn't carry, so a local-time punch card isn't derivable — and bucketing in
  whatever timezone the *server* runs in would make the chart's meaning shift
  with the host's `TZ`. The endpoint returns the timezone as a field so the UI
  can't imply otherwise.
- **Shallow clones are surfaced as a caveat**, not silently under-reported.
  Verified live on this repo, which *is* a shallow clone: all 133 commits land
  in the current week — an artifact of truncated history, not a finding. That's
  precisely the case where a trend chart looks fine and is wrong.
- **`commit_activity` is pure and takes `now` as a parameter**, so it needs no
  repo and no clock and is deterministically testable.
- Cell opacity carries magnitude while each cell's `<title>` carries the count,
  so the value is never colour-only.
- 7 new tests (6 in `repowise-git` — UTC bucketing, week ordering, commits
  outside the trend window still counted on the card, empty input, negative and
  far-future timestamps; 1 server test for the no-history empty state).

---

## PR #260 — UI: debounce search, prompt on empty, clickable symbol results
**2026-07-28** · closes [#260](https://github.com/baileyrd/rusty_repo_wise/issues/260)

- **Smaller than the issue described, and the issue was wrong.** #260 assumed
  no search UI existed. A `SearchBox` was already there with Ctrl/Cmd+K focus
  and live results, so this fixes what it actually lacked rather than building
  a second one.
- **Debounced by 200ms.** Every keystroke previously issued an HTTP request —
  typing "parser" fired six, five obsolete before they returned. A further
  keystroke re-runs the resource and drops the in-flight future before the
  delay elapses, so only a pause in typing queries. `gloo-timers` was already
  a dependency, so no new crate.
- **An empty box now prompts** instead of rendering nothing. Silence reads as
  "no matches" when you haven't typed yet — a different and discouraging
  message. `should_query` guards the request explicitly rather than relying on
  the server returning an empty result for an empty needle.
- **Symbol results are links**, matching file results. They were plain text,
  and a result you can't act on is half a result.
- **Result counts** are shown, so a truncated-looking list isn't mistaken for
  the whole answer.
- 2 new tests (10 total in the WASM crate). The debounce bound is enforced by a
  **compile-time** `const _: () = assert!(...)` rather than a test — clippy
  flagged asserting on a constant and its suggestion was the better design: the
  constraint is on the constant, so raising it into perceptible-lag territory
  should fail the build, not a test run. Verified it bites by setting it to
  5000ms and watching `cargo check` fail.
- **Correction to an earlier claim:** the UI gap analysis said this port had no
  ⌘K command palette. Ctrl/Cmd+K focus already existed — what's still missing
  is a palette that *navigates*, which needs routing (#259).

---

## PR #261 — UI: files treemap, and the WASM crate's first tests
**2026-07-28** · closes [#261](https://github.com/baileyrd/rusty_repo_wise/issues/261)

- **Added:** `GET /api/files` and a `FilesSection` rendering an SVG treemap —
  area ∝ lines, fill by health band. Third issue of the UI parity round.
- **My own issue scoped this wrong.** #261 said "frontend only, over existing
  endpoints", but `/api/health` returns only the **worst 15** files
  (`WORST_FILES_LIMIT`) and nothing exposed per-file lines for the whole repo.
  A whole-repo treemap needs a whole-repo endpoint, so this adds one.
- **Hand-written squarified treemap** (~60 lines) rather than a charting
  dependency, which a WASM binary shouldn't grow for this. Slice-and-dice was
  rejected: it degenerates into unreadable slivers well before 85 files.
- **`unscored` is its own band.** `score` is `Option<f64>` end to end rather
  than defaulting to 10.0 — a file with no health score is not a healthy file,
  and coloring unknown risk green is worse than not coloring it.
- **Color is not the only channel:** every tile names its band in an SVG
  `<title>`, and the legend names the bands rather than only showing swatches.
- **`health_available: false` degrades honestly** to sized-but-uncolored tiles
  with a stated reason, rather than rendering everything grey unexplained.
- **The WASM crate had no tests at all** — it turns out it compiles fine for the
  host target, so its pure logic was always testable and simply never tested.
  Adds 8 tests (area conservation, proportionality, in-bounds tiles,
  determinism, degenerate inputs, band boundaries) and a `Test (WASM web
  crate)` CI step alongside the fmt/clippy steps added in #256.

---

## PR #258 — UI: contributors view (`/api/contributors` + `ContributorsSection`)
**2026-07-28** · closes [#258](https://github.com/baileyrd/rusty_repo_wise/issues/258)

- **Added:** `GET /api/contributors` and a `ContributorsSection`. Second issue
  of the UI parity round, surfacing `bus_factor` (#239), which until now was
  CLI-only and one file at a time.
- **Reports per-author owned lines, share, and files touched**, plus the repo's
  distribution of per-file bus factors — the concentration question the
  existing per-file `ownership` output couldn't answer at a glance.
- **Bus factor rendered in words, not as a bare number.** "1" reads to some as
  "one clear owner, tidy" — the opposite of its meaning. The CLI already spells
  it out for this reason and the UI must not regress it.
- **Bounded, not cached.** `ownership_of` shells out to `git blame` once per
  file, so an unbounded sweep is one subprocess per indexed file. The sweep is
  capped at the 200 largest files: a cache would need an invalidation story
  (the index has one, git history doesn't), whereas a bound is stateless and
  its cost is knowable up front.
- **End-to-end verification caught a misleading message in the first cut.** The
  response originally implied "bounded sample" whenever `files_sampled <
  files_total` — but on this repo that gap is **21 unblameable files** (untracked
  or never committed), with the 200-file bound never applying at 85 files. Those
  are different facts, so `limit_applied` and `files_unblameable` are now
  reported separately and the view phrases each correctly.
- 1 server test (no-git-history reports unavailable while still reporting the
  real index size, so the UI can tell "no git" from "empty repo").

---

## PR #257 — UI: coverage view (`/api/coverage` + `CoverageSection`)
**2026-07-28** · closes [#257](https://github.com/baileyrd/rusty_repo_wise/issues/257)

- **Added:** `GET /api/coverage` in `repowise-server` and a `CoverageSection`
  in `crates/repowise-web`. First issue of the UI parity round.
- **Closes a loop opened earlier today.** #241/#242/#243 built coverage ingest,
  impacted-tests, and two health markers — and nothing surfaced any of it. The
  data was computed, stored at `.repowise/coverage.json`, and reachable only
  from the CLI.
- **The API keeps "never measured" and "0% covered" apart.** Measured files go
  in `files`; indexed files no report mentioned are counted in
  `unmeasured_files` rather than listed at 0%. `CoverageData::line_coverage_of`
  returns `None` vs `Some(0.0)` precisely to preserve that, and flattening them
  at the API boundary would have quietly undone it.
- **The view states the unmeasured count rather than implying coverage.**
  Without that line the measured set reads as the whole repo — on this repo,
  1 measured file out of 85 would look like complete coverage.
- **Reports per-test map presence**, since `repowise impacted-tests` can't run
  without one and that's invisible otherwise.
- **Coverage on disk that matches no indexed file reports `available: false`**
  rather than "0 measured files", which would read as "nothing is covered".
- 2 server tests. Verified end-to-end against a live `serve-dashboard`: both
  the nothing-ingested and the one-file-at-50% states.

---

## PR #256 — One frontend: remove the React app, give the Leptos crate a real CI gate
**2026-07-28**

- **Removed:** the React/Vite app under `web/` (37 files). It arrived in the
  direct-push commit `894d0a6` and duplicated `crates/repowise-web` against the
  same `/api/*` surface, with *more* views — but it was absent from CI, from
  the Cargo workspace, from the README, and from the server's `--static-dir`
  wiring, and its README was the unmodified stock Vite template. Nothing built,
  type-checked, linted, or tested it, so it could rot or break silently. The
  README meanwhile stated the Leptos crate was "chosen deliberately over a real
  Next.js/React frontend" — the repo held a documented decision and a working
  contradiction of it at the same time.
- **Nothing outside `.repowise/` (gitignored) referenced `web/`**, so the
  removal touches no build, no config, and no other crate.
- **Fixed a gap the removal exposed:** `crates/repowise-web` is deliberately
  outside the root workspace (it only targets wasm32 — putting it in would
  break `cargo build/test/clippy --workspace` for every host-target crate), and
  the consequence was that CI's workspace-wide `Format`/`Clippy`/`Test` steps
  **skipped it entirely**. Its only gate was a bare `cargo check`: no
  formatting check, no lints. The surviving frontend was held to a weaker
  standard than every other crate in the repo.
- **Added `Format (WASM web crate)` and `Clippy (WASM web crate)` CI steps**,
  run with `--manifest-path` against `wasm32-unknown-unknown`. Clippy compiles
  as it lints, so these subsume the bare `cargo check` they replace. Both
  already pass, so this locks in the current state rather than papering over
  a break.
- README's `repowise-web` section now states it is the single frontend, why the
  React app was removed, and why the crate needs its own CI steps.

---

## PR #255 — Parity: architecture-model export in JSON Graph Format
**2026-07-28** · closes [#244](https://github.com/baileyrd/rusty_repo_wise/issues/244)

- **Added:** `repowise_graph::json_graph` and `repowise export --format
  json-graph`, writing the dependency graph to `<DIR>/architecture.json`.
  Completes #244 — `--format markdown` (PR #254) remains the default.
- **JGF was chosen over DOT and Mermaid** because it's the only one of the
  three that carries per-node metadata losslessly. The graph's nodes know their
  language, line count, symbol kind, complexity, nesting depth, and parent
  type; DOT and Mermaid would discard all of it to render a picture. Serde is
  already a workspace dependency, so this adds no new third-party crate.
- **The export is portable.** Paths are repo-relative with forward slashes, and
  symbol ids are rebuilt from the relative path rather than reusing
  `Symbol::id` — which embeds an **absolute** path and would otherwise bake the
  producing machine's directory layout into every id. A test caught exactly
  that leak during development.
- **Output is deterministic** (nodes in a `BTreeMap`, edges sorted), so two
  exports of an unchanged repo are byte-identical and therefore diffable.
- **The graph is partial, and says so.** Unresolved imports and calls have no
  target node and therefore no edge. Rather than emit something that merely
  *looks* complete, `graph.metadata.unresolved` reports the counts, names the
  import stems that failed to resolve, and carries a note that absent edges do
  not imply absent dependencies — and the command prints the same warning to
  the terminal. On this repo that's **381 unresolved imports and 7804
  unresolved calls**, which a reader drawing conclusions from the edges needs
  to know.
- **Re-exporting over a previous `architecture.json` doesn't need `--force`**,
  though an unrelated file in the target still triggers the refusal. Demanding
  `--force` for the ordinary re-run would train people to pass it habitually,
  which is exactly when it stops protecting anything.
- 10 new tests (7 in `repowise-graph` over a real parsed fixture with a
  genuinely unresolvable external import, 3 in `repowise-cli` for the target
  guard).

---

## PR #254 — Parity: `repowise export` (markdown half), and fix the wiki page count
**2026-07-28** · part of [#244](https://github.com/baileyrd/rusty_repo_wise/issues/244)

- **Added:** `repowise export --out <DIR> [PATH]`, copying pages generated by
  `repowise docs` out of `.repowise/wiki/` into a target directory with the
  tree preserved. Also adds `repowise_docs::wiki_root`, exposed for the same
  reason `wiki_page_path` already was — so consumers don't re-derive the
  `.repowise/wiki` convention.
- **Deliberately does not close #244.** The reference's one-line `export`
  covers two features: wiki pages *or* an "architecture model". Only the first
  is implemented; which interchange format the second should use (DOT?
  Mermaid? JSON Graph?) is still an open design question, so the issue stays
  open for it.
- **A non-empty target is refused without `--force`.** Export targets are
  frequently something like `./docs`, and quietly merging into or partly
  overwriting a hand-written docs tree would be destructive and hard to undo.
  A test asserts a pre-existing file survives the refusal byte for byte, and
  survives a subsequent `--force` run too.
- **An empty result is an error, not a silent success** — "exported 0 pages"
  would be indistinguishable from a real export of a repo with no docs.
- **Fixed a bug introduced by PR #247:** `repowise status` reported
  `wiki: no pages` on a repo with **85 generated pages**. `count_wiki_pages`
  read only the top level of `.repowise/wiki`, but `repowise docs` mirrors the
  repo's own tree there — so a project whose sources live in subdirectories
  (i.e. essentially all of them, including this one) has *nothing* at the wiki
  root. It now shares `export`'s recursive walk, so the two can't disagree
  about what counts as a page.
- 7 new tests, the first of which pins the recursive walk against a fixture
  asserted to have no top-level pages — the exact shape that hid the bug.

---

## PR #253 — Parity: coverage health markers (`coverage_gap`, `untested_hotspot`)
**2026-07-28** · closes [#243](https://github.com/baileyrd/rusty_repo_wise/issues/243)

- **Added:** `FindingKind::CoverageGap` (−0.4) and
  `FindingKind::UntestedHotspot` (−1.0), plus a new `analyze_with_context`
  entry point. Ninth and final implementable issue of the parity round.
- **`untested_hotspot` needs three independent signals to agree** — the file
  is a churn hotspot, four or more files depend on it, and it's under 40%
  covered. Each alone is unremarkable: a well-tested hotspot is fine, an
  untested leaf nobody imports is fine. The intersection is where risk
  concentrates, which is the same reasoning behind `hot_path_sync_io` (#186)
  and why this can carry the heavier weight without flooding scores.
- **The two markers never both charge one file.** `untested_hotspot` subsumes
  `coverage_gap`, so a file earning the former skips the latter — stacking
  both would double-penalize a single underlying fact.
- **Without ingested coverage, neither marker fires.** A repo that never ran
  `coverage add` must not be scored as untested, and that guarantee runs down
  to `line_coverage_of` returning `None` for an unmeasured file versus
  `Some(0.0)` for one measured with nothing executed. The first test in the
  new suite pins it across all three entry points.
- **Follows #186's precedent for extra signals:** the caller supplies
  coverage, and `analyze`/`analyze_with_weights`/`analyze_with_hotspots` all
  behave exactly as before and simply never see these markers. `CoverageData`
  lives in `repowise-core`, which `repowise-health` already depended on, so
  this adds **no new dependency**.
- **Both penalties are `--weights`-configurable** like every other marker, and
  all four constants (two thresholds, one dependent count, two penalties) carry
  their rationale at the definition.
- **Still fixed-penalty.** Nothing here presumes an answer to #62's
  ML-calibrated-weights question, which stays open.
- Writing the tests surfaced that `Language::Other` files never get import
  edges at all (`repowise-graph` skips them), so a fixture using it can never
  produce dependents — noted in the test so the next author doesn't repeat it.
- 6 new tests in `repowise-health`.

---

## PR #252 — Parity: add `repowise impacted-tests`
**2026-07-28** · closes [#242](https://github.com/baileyrd/rusty_repo_wise/issues/242)

- **Added:** `repowise_git::changed_lines` (line-level diff extraction) and
  `repowise impacted-tests [REVSPEC]`, intersecting a diff's changed lines with
  the per-test coverage map from #241. Eighth issue of the parity round.
- **Line-level, not file-level.** `change_risk` already resolved a revspec to
  changed *files* via `--numstat`; impacted-test selection needs changed
  *lines*, so this parses `git diff -U0` hunk headers. Deletion-only hunks
  contribute nothing, which is correct — a coverage map records lines that
  exist and ran, and a deleted line does neither.
- **The safeguard is the feature.** Four distinct situations yield an empty
  test list and only one means "no test is affected": `MapPresent`, `NoMap`,
  `NoCoverage`, and `NoSourceLineChanges`. The three that *cannot* answer print
  `CANNOT ANSWER` and say why. A developer who reads "no impacted tests" as
  "safe to skip testing", when the real cause was that no coverage was ever
  ingested, has been actively misled into shipping untested code.
- **Even the genuine empty result is hedged** — it states the change is
  untested *by the ingested suite*, not that it's safe.
- **One predicate owns the distinction.** `Status::empty_list_is_meaningful()`
  is the single place deciding whether an empty list is a finding or a hole in
  the data, and `render` branches on it rather than re-deriving that per
  variant.
- **End-to-end verification found a real trap:** `git show` reports no diff at
  all for a **merge commit**, so running this on a merge read as "this change
  touched nothing". An entirely empty diff is now its own message naming the
  merge-commit cause and suggesting a parent or a `base..head` range.
- 12 new tests (5 in `repowise-git` for diff parsing, 7 in `repowise-cli` for
  selection and every output state).

---

## PR #251 — Parity: test-coverage ingest (`repowise coverage add|status`)
**2026-07-28** · closes [#241](https://github.com/baileyrd/rusty_repo_wise/issues/241)

- **Added:** `repowise_core::coverage` (LCOV parser + `CoverageData` store) and
  `repowise coverage add|status`. Seventh issue of the parity round, and the
  first slice of a layer this port had **nothing** for — before this, the
  string `coverage` appeared once in the whole crate tree, in an unrelated
  comment.
- **Stays deterministic.** The reference describes test intelligence as
  index-lookup based with no LLM and no network, so this lands entirely inside
  the port's existing design — it doesn't touch #61's open LLM question.
- **Stores both shapes the reference defines:** a per-file aggregate merged
  across tests, and a per-test map (from LCOV `TN:` records) of which test
  executed which lines. The per-test map has no consumer until #242, but it's
  recorded now — re-ingesting every report later just to add it would be
  wasted work, and LCOV supplies it for free.
- **LCOV only, deliberately.** Cobertura XML, Clover XML, coverage.py SQLite,
  and the reference's normalized JSON would each pull in an XML or SQLite
  dependency. LCOV is the one format parseable with nothing new.
- **Reports merge rather than replace** (`--replace` opts out), so a suite
  sharded across CI jobs can be ingested one report at a time.
- **"Never measured" is kept distinct from "0% covered."**
  `line_coverage_of` returns `None` for a file no report mentions and
  `Some(0.0)` for one measured with nothing executed. Collapsing them would
  make unmeasured files indistinguishable from untested ones — the exact error
  that would poison #243's `untested_hotspot` marker.
- **Unmatched paths are reported loudly, not dropped.** LCOV records paths from
  whatever machine ran the suite, so ingest resolves against the repo root and
  retries by longest suffix (`/builds/proj/src/lib.rs` still finds the local
  `src/lib.rs`). Anything still unresolved is warned about by count and by
  name: coverage that silently matched nothing looks exactly like coverage that
  worked.
- **Malformed input errors rather than panicking** — a bad `DA:` record, or one
  appearing before any `SF:`, is a clear error; `LF:`/`LH:`/`FN*`/`BR*` summary
  records are ignored rather than rejected, since older writers vary.
- 11 new tests in `repowise-core`.

---

## PR #250 — Parity: add `repowise doctor` (setup diagnostics)
**2026-07-28** · closes [#240](https://github.com/baileyrd/rusty_repo_wise/issues/240)

- **Added:** a `repowise-cli::doctor` module and `repowise doctor [PATH]`,
  reporting each check as `pass`/`warn`/`FAIL` with a remedy line. Sixth issue
  of the parity round.
- **Why it's worth having here:** this port has many environment-dependent,
  degrade-softly paths, and each previously surfaced only when you happened to
  run the command that needed it. Checks the `git` binary, whether the
  directory is a git repo, clone depth, index presence, and both optional env
  vars — naming exactly what each one degrades to when unset.
- **A degraded setup warns, it never fails**, and only a hard failure exits
  nonzero. Missing an optional token is not an error; reporting it as one would
  train people to ignore `doctor` and make it useless in a CI gate that only
  cares about real breakage.
- **The shallow-clone check is the most valuable one.** A shallow clone doesn't
  make `hotspots`/`coupled`/`risk` *fail* — it makes them quietly under-report,
  which is much harder to notice. It's skipped entirely outside a git repo,
  since reporting "full history" for a directory with no git at all would be a
  misleading pass.
- **This caught a real condition on first run:** the checkout it was verified
  against is itself a shallow clone, so its git-history numbers had been
  under-reporting unnoticed.
- Diagnostic only — no state is mutated.
- 6 new tests (28 total in `repowise-cli`).

---

## PR #249 — Parity: add `repowise hook install|uninstall|status`
**2026-07-28** · closes [#238](https://github.com/baileyrd/rusty_repo_wise/issues/238)

- **Added:** a `repowise-cli::hook` module and `repowise hook
  install|uninstall|status`, writing a `post-commit` hook that refreshes the
  index after each commit. Fifth issue of the parity round.
- **One of the reference's five auto-sync mechanisms, chosen deliberately.**
  The reference drives auto-sync through a post-commit hook, a file watcher,
  GitHub and GitLab webhooks, and polling. The hook is the only one needing
  **no new dependency, no daemon, and no server**. `repowise watch` would
  require a filesystem-notification crate — a new third-party dependency, and
  therefore a stop-and-ask rather than something to add unattended.
- **The hook runs `repowise update` detached and silenced.** Git waits for
  `post-commit` to exit, so a slow re-index would stall every commit. Auto-sync
  that made `git commit` hang would be worse than no auto-sync.
- **A hook this tool didn't write is never touched.** `.git/hooks/post-commit`
  is somewhere users and other tools legitimately put things, so the body
  carries a marker line and only its presence authorizes an overwrite or a
  delete. Anything else classifies as `Foreign`: `install` and `uninstall` both
  refuse, `status` says so, and a test asserts the foreign hook survives both
  calls byte for byte.
- **Re-installing our own hook is idempotent**, not an error — it refreshes the
  script if the hook body has since changed.
- **A worktree or submodule is an explicit error.** When `.git` is a *file*
  rather than a directory, the real hooks live in the parent repo; creating
  `.git/hooks` there would silently produce a hook git never runs. Reporting
  that beats reporting success for something that will never fire.
- The executable bit is set on Unix, and skipped on Windows with a comment
  explaining why (git's bundled shell doesn't consult it).
- 6 new tests (22 total in `repowise-cli`).

---

## PR #248 — Parity: add bus factor to `repowise ownership`
**2026-07-28** · closes [#239](https://github.com/baileyrd/rusty_repo_wise/issues/239)

- **Added:** `repowise_git::bus_factor(&[Ownership]) -> usize`, reported by
  `repowise ownership`. Fourth issue of the parity round.
- **Costs no extra git work.** It's a pure function over the per-author line
  shares `ownership_of` already returns from `git blame --line-porcelain` — no
  new invocation, no new data collection.
- **Answers what `ownership` couldn't.** The existing output shows *who* owns a
  file but not whether that ownership is dangerously concentrated: one author
  at 95% and four authors at 25% each produced the same shape of output.
- **Threshold is 50%, and the choice is documented.** The reference defines
  "bus factor" in its computed glossary but doesn't publish its threshold, so
  this picks a simple majority — the smallest set of people who between them
  wrote most of the file. A higher bar (80%) would answer the different, less
  actionable question "who wrote nearly all of it".
- **Reported in words, not as a bare number.** "bus factor: 1" reads to some as
  "one tidy owner", the exact opposite of the meaning, so the output spells it
  out: `1 -- one author wrote most of this file`.
- **Two degenerate cases distinguished:** a file with no blameable lines
  returns `0` and prints `n/a`, which is deliberately *not* the same as a bus
  factor of `1`. Shares that never reach the threshold (partial blame
  attribution, rounding loss) fall back to counting every author rather than
  silently returning a too-low number.
- **Doesn't assume its input is sorted** — `ownership_of` returns
  highest-share-first, but `bus_factor` sorts defensively, with a test pinning
  that.
- 6 new tests (5 in `repowise-git`, 1 in `repowise-cli`).

---

## PR #247 — Parity: add `repowise status` (index freshness)
**2026-07-28** · closes [#237](https://github.com/baileyrd/rusty_repo_wise/issues/237)

- **Added:** `repowise status [PATH]` with `--verbose`. Third issue of the
  parity round.
- **Scoped deliberately against `overview`.** The reference's `status` reports
  "wiki sync state, page statistics, and coverage" — but `repowise overview`
  already covers file/symbol/language counts here, so duplicating them would
  have made two commands that mostly agree. This reports the half `overview`
  can't: whether the index still *describes* the tree on disk.
- **Reports:** indexed file count, how many indexed files were modified or
  deleted since indexing, and whether wiki pages and a dashboard exist.
- **Staleness is filesystem-based, not git-based** — each indexed file's mtime
  against the index's own. That works with no git history, a shallow clone, or
  no git at all, and it catches uncommitted edits a diff against the indexed
  commit would miss.
- **The blind spot is printed, not hidden.** This approach can't see files
  *created* since indexing (that needs the full re-walk `repowise update`
  does), so the command says so in its own output. A freshness check that
  quietly missed new files would be worse than one that admits the limit.
- **No index is a state, not an error** — "index: none, run `repowise init`",
  exit 0.
- Coverage stats, which the reference folds into `status`, are deliberately
  left out until the coverage layer (#241) lands.
- 5 new tests (15 total in `repowise-cli`); `render_status` is pure, so every
  branch is testable without touching disk.

---

## PR #246 — Parity: expose change-risk scoring as `repowise risk`
**2026-07-28** · closes [#236](https://github.com/baileyrd/rusty_repo_wise/issues/236)

- **Added:** `repowise risk [REVSPEC] [PATH]`, scoring a single commit or a
  `base..head` range (default `HEAD`). Second issue of the parity round.
- **Same shape as #235:** `repowise_git::change_risk` was already public and
  already drove the `get_change_risk` MCP tool (#42). An agent could ask for a
  change-risk score over MCP; a human at a terminal couldn't — despite this
  being one of the reference's headline pre-commit workflows. One scoring
  path, now with two surfaces.
- **Reports the full diff shape, not just the number:** files, lines
  added/deleted, subsystems touched, concentration entropy (with its 0.00/1.00
  endpoints spelled out inline), and the head commit's author plus their prior
  commit count — the inputs the score is built from, so a surprising score can
  be traced rather than just trusted.
- **Added a `low`/`moderate`/`high` band** at 4.0 and 7.0. Explicitly
  presentational: the note printed under every result restates that the score
  is a fixed-weight heuristic, not a calibrated probability. The bands are
  round numbers, not corpus-derived thresholds, and that's documented at the
  function.
- **`render_risk` and `risk_band` are pure functions**, tested without a git
  repo present (4 new tests, 10 total in `repowise-cli`).

---

## PR #245 — Parity: expose dead-code detection as `repowise dead-code`
**2026-07-28** · closes [#235](https://github.com/baileyrd/rusty_repo_wise/issues/235)

- **Added:** `repowise dead-code [PATH]`, with `--min-confidence
  <low|medium|high>` and `--limit <N>`. First issue of the parity round
  against `repowise-dev/repowise`'s `docs/reference/CLI_REFERENCE.md`.
- **No new analysis.** `repowise_health::find_dead_code` was already public
  and already drove the `get_dead_code` MCP tool (#45) and the dashboard —
  it just had no CLI surface. Every other analysis layer in this port
  (`health`, `hotspots`, `ownership`, `coupled`, `deps`) had a command;
  this closes an inconsistency in the port's own surface as much as a gap
  against the reference.
- **`--min-confidence` deliberately mirrors the MCP tool's argument** of
  the same name, parsed through one shared `parse_min_confidence` so the
  two surfaces can't accept different spellings over time.
- **Rendering split into a pure `render_dead_code` function**, so filtering,
  truncation, and the empty case are testable without an index on disk.
  This adds the first `mod tests` to `repowise-cli` (6 tests) — the crate
  previously had none, since its commands print inline.
- **Truncation is stated, never silent:** the full matching count is printed
  before the capped list, and an "N more not shown" line follows it, so a
  truncated result can't be misread as a complete one.
- **An empty result is a clean bill, not an error** — "no candidates at or
  above the requested confidence tier", exit 0.
- **Documented a precision caveat found while verifying end-to-end:** the
  analysis doesn't exclude `#[test]` functions, which have no in-repo
  callers by construction and so dominate the `high` tier (391 candidates
  on this repo, mostly tests). Excluding them would change `get_dead_code`'s
  results too, so it's called out in the README rather than silently changed
  here.

---

## Restore green CI: fix `cargo fmt` violations on `main`
**2026-07-28**

- **Fixed:** two `cargo fmt` violations in `repowise-parser` (`go.rs`
  `param_type`, `kotlin.rs` `computes_primitive_param_count`) that landed with
  the direct push below and turned `main` red. The CI `check` job runs Format
  first and gates the rest on it, so Clippy, Test, and the WASM check were all
  **skipped** — the break masked the entire suite rather than just one step.
  With formatting restored, all four run and pass (411 tests, Clippy clean,
  `repowise-web` builds for `wasm32-unknown-unknown`).
- **Fixed:** the entry below was keyed to **PR #233**, but #233 was the
  release-notes PR for #231/#232. The change it describes was pushed directly
  to `main` and had no PR, so it is now keyed by commit like the other two
  pre-PR entries.
- **Fixed:** a `file:///c:/dev/...` absolute Windows path in that entry,
  replaced with a repo-relative path.
- **Added:** the `web/` React frontend to that entry — the commit's own subject
  names it, but the notes omitted it entirely.

---

## 2026-07-26 — Extend primitive parameter extraction to Java, Kotlin, Go, C++, C#; add WASM CI step; add MCP mtime caching; add web frontend
[`894d0a6`](https://github.com/baileyrd/rusty_repo_wise/commit/894d0a6b7cc91acb9ff1e187c412900bbbb9d4b7)

- **Added:** primitive parameter type extraction (`primitive_param_count`) in `repowise-parser` for **Java**, **Kotlin**, **Go**, **C++**, and **C#** (`java.rs`, `kotlin.rs`, `go.rs`, `cpp.rs`, `csharp.rs`), extending `primitive_obsession` health marker detection in `repowise-health` across all major statically-typed languages.
- **Added:** 5 new `repowise-parser` unit tests verifying primitive parameter count extraction across Java, Kotlin, Go, C++, and C#.
- **Added:** CI verification step in `.github/workflows/ci-rust.yml` to compile and validate `crates/repowise-web` against target `wasm32-unknown-unknown`.
- **Added:** Thread-safe `mtime`-validated in-memory index and dependency graph caching in `RepowiseServer` (`crates/repowise-mcp/src/lib.rs`), avoiding redundant disk reads and petgraph construction across sequential MCP tool calls when `.repowise/index.json` is unchanged.
- **Added:** `repowise-mcp` unit test verifying `mtime` caching behavior across `load()` calls.
- **Added:** a React/Vite frontend under `web/` (sidebar plus Overview, Architecture, Graph, Health, Hotspots, Commits, Decisions, Docs, Wiki, Usage, Dead Code, Workspace, Chat, and Settings tabs). Note this is a **second, separate** web surface from the Leptos/WASM `crates/repowise-web` crate that the new CI step checks.
- **Note:** this change was pushed directly to `main` rather than landing through a PR, contrary to the workflow in CLAUDE.md and CONTRIBUTING.md, and it left `main` failing CI on formatting until the fix above.

---

## PR #232 — Add hot_path_sync_io health marker (completes issue #72)
**2026-07-25** · [#232](https://github.com/baileyrd/rusty_repo_wise/pull/232) · closes [#186](https://github.com/baileyrd/rusty_repo_wise/issues/186)

- **Added:** `hot_path_sync_io`, the **nineteenth and final** slice of
  issue #72's Performance-signal cluster. Flags a synchronous, blocking
  I/O call in a function whose file the git hotspot analytics rank among
  the repo's churn-and-complexity-heaviest.
- **The only marker built from two independent signals:** a structural
  one (a blocking call is present) and an empirical one (git says this
  file changes often). Neither alone is a finding — a blocking read in a
  rarely-run setup path is fine, and a hotspot with no blocking I/O has
  nothing to fix. The intersection is far higher-precision than either
  half.
- **`Symbol` gains `sync_io_calls: Vec<SyncIoCallRef>`** (each entry:
  `line`, `callee_name`) — every I/O-shaped call anywhere in the body,
  not just inside a loop like `io_in_loop`.
- **How git data gets in without compromising `repowise-health`.** The
  crate is a pure function of the index and the call graph and knows
  nothing about git; taking a git dependency to serve one marker would
  be a bad trade. The *caller* computes the hot-file set and passes it
  in as plain paths, through a new `analyze_with_hotspots(index, graph,
  weights, hot_files)`. `analyze`/`analyze_with_weights` still exist and
  delegate with an empty set, so docs/dashboard/MCP/server compile and
  behave exactly as before and simply never see this marker. A test pins
  that the no-git path reports nothing here.
- **"Hot" is a relative rank, not an absolute score** — hotspot scores
  are churn × complexity and aren't comparable between repos. The set is
  the top 10 files, *further capped to a quarter of the repo*, excluding
  anything scoring zero.
- **That second cap came out of end-to-end verification**, not design: a
  plain top-10 marked every file in a two-file repo as hot, silently
  degrading the marker into "any sync I/O anywhere" and discarding the
  empirical half of the signal it exists for.
- **Fails soft:** no git history (no repo, shallow clone, git missing)
  yields an empty set and the marker quietly doesn't fire. Losing one
  marker beats refusing to score the codebase.
- **Call recognition reuses `io_in_loop`'s callee table verbatim**; only
  the scope changes, a whole-body scan rather than a loop-body one.
- **New `FindingKind::HotPathSyncIo`** (penalty −0.3), the
  per-occurrence tier alongside `io_in_loop`: a single blocking call is
  a real but bounded cost that doesn't worsen with input size, and the
  precision comes from the hotspot gate rather than a heavier weight.
- **Verified end to end** against a real git repo: two files with
  identical blocking `open().read()` + `json.loads`, one churned nine
  times and one committed once. Only the churned file is flagged.
- **Scope:** Rust, Python, and TypeScript/JavaScript.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update`.
- **Issue #72 is now complete** — all 19 Performance-signal sub-issues
  are implemented, and the health scorer covers 31 distinct markers.

---

## PR #231 — Add membership_test_in_loop health marker
**2026-07-25** · [#231](https://github.com/baileyrd/rusty_repo_wise/pull/231) · closes [#182](https://github.com/baileyrd/rusty_repo_wise/issues/182)

- **Added:** `membership_test_in_loop`, the eighteenth slice of issue
  #72's Performance-signal cluster. Flags an `x in xs` /
  `xs.contains(&x)` / `xs.includes(x)` test inside a loop body where
  `xs` is a list. Each test scans the whole list, so one per iteration
  makes the loop O(n × m) while it reads as linear.
- **`Symbol` gains `membership_test_in_loop: Vec<MembershipTestInLoopRef>`**
  (each entry: `line`, `collection`).
- **The design call the issue asked for.** #182's own acceptance
  criteria flagged that recognizing a list-typed collection "needs at
  least approximate type information … may need its own scoping/design
  pass on how far to push type inference." The answer taken here:
  **don't build type inference.** Instead use the narrow slice reliably
  visible in one function's own text — a local binding whose initializer
  shape names the collection kind outright (`xs = [..]`,
  `let xs = vec![..]`, `const xs = new Set(..)`). A first pass collects
  those into a name → kind map; the loop walk only flags a test that
  resolves to a list.
- **Deliberately low-recall, high-precision.** A parameter, a field, an
  imported constant, or any unclear initializer never enters the map and
  is skipped. A false positive here tells someone to "fix" a lookup that
  was already O(1), which is worse than staying quiet. A name rebound to
  a different kind is demoted for the same reason — one pass can't know
  which binding is live at the test site.
- **Rust is where the binding map earns its keep:** `Vec::contains` and
  `HashSet::contains` are spelled identically at the call site, so
  nothing local separates the O(n) scan from the O(1) lookup. There the
  *declared type* wins over the initializer, which is what makes
  `let seen: HashSet<_> = xs.into_iter().collect();` resolvable when
  `.collect()` alone says nothing.
- **Two things fall out for free in JS/TS:** `Set.has` is never a
  membership target (it's already the recommended form), and substring
  checks need no special case — strings share `includes`/`indexOf` with
  arrays, but a string binding never resolves to a list.
- **New `FindingKind::MembershipTestInLoop`** (penalty −0.6), the
  quadratic-*shape* tier alongside `nested_loop_quadratic`.
- **Testing:** list vs. set/dict in all three languages, plus `not in`,
  inline list literals, unknown bindings, rebinding, the Rust
  declared-type case, and the JS string receiver — plus a health-side
  per-occurrence test and a `HealthWeights::default()` assertion.
- **Verified end to end** on a Python and a JS fixture: only the
  list-bound collection is flagged in each; the set, dict, parameter,
  and string receivers stay quiet.
- **Scope:** Rust, Python, and TypeScript/JavaScript.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update`.

---

## PR #229 — Fix defer_in_loop flagging defers inside func literals
**2026-07-25** · [#229](https://github.com/baileyrd/rusty_repo_wise/pull/229)

- **Fixed:** `defer_in_loop` (#189, shipped in #226) flagged a `defer`
  inside a `func() {...}` literal whose loop sat outside it. A Go
  `defer` runs when the innermost enclosing *function* returns, and a
  literal is such a function even though it gets no `Symbol` of its own
  in this port — so those defers already run at the end of their own
  iteration and were never a leak.
- **Why it mattered more than an ordinary false positive:** wrapping a
  loop body in a function literal is precisely the fix the finding
  recommends, so the marker penalized its own remedy. It also fired on
  `defer wg.Done()` inside a goroutine's literal, which appears in
  essentially every concurrent Go codebase. Caught while verifying
  #228's end-to-end fixture, where the `WaitGroup` fan-out was reported
  as a defer leak.
- **The fix:** `defers_in_loops` takes an `is_defer_scope` classifier
  (Go: `func_literal`) and resets the "inside a loop" state on entering
  one. Deliberately a *different* predicate from `is_nested_function`,
  which decides what gets its own `Symbol` — a literal is not a symbol
  boundary but it is a defer-scope boundary, and conflating the two is
  what produced the bug.
- **Recursion is preserved:** the walk still descends into literals, so
  a loop nested inside one keeps its defers flagged, attributed to the
  enclosing named function. Both halves are pinned by tests, replacing
  the earlier test that had encoded the wrong behavior as correct.
- No `Symbol` field changes, so no index re-`init` is needed for this
  one.

---

## PR #228 — Add goroutine_in_unbounded_loop health marker (Go)
**2026-07-25** · [#228](https://github.com/baileyrd/rusty_repo_wise/pull/228) · closes [#190](https://github.com/baileyrd/rusty_repo_wise/issues/190)

- **Added:** `goroutine_in_unbounded_loop`, the seventeenth slice of
  issue #72's Performance-signal cluster. Flags a Go `go` statement
  inside a loop body whose only limit on concurrency would be the
  iteration count. One goroutine per input item works fine in testing
  with a handful of items and fails only in production at scale.
- **`Symbol` gains
  `goroutine_in_unbounded_loop: Vec<GoroutineInUnboundedLoopRef>`** (each
  entry: `line`, `callee_name`).
- **The "unbounded" qualifier is the whole design.** A loop counts as
  bounded when its body contains a channel send (`sem <- struct{}{}`) or
  receive (`<-tokens`) — the acquire half of Go's standard semaphore and
  worker-pool idioms, and the only mechanism recognized here.
  `sync.WaitGroup` deliberately does *not* suppress, per the issue's own
  requirement: `wg.Add`/`wg.Wait` bound how *completion* is tracked, not
  how many goroutines run at once.
- **The bound scan skips the launched goroutine's own subtree.** In
  `go func() { results <- work(v) }()` the channel send is the goroutine
  reporting its own result, not the loop throttling how many exist;
  counting it would suppress precisely the case worth flagging. This is
  the difference between a useful marker and one that silently never
  fires, and a test pins it.
- **Suppression is scoped per loop and inherited inward** — an inner
  loop can't un-bound the semaphore an enclosing loop already acquired.
  That per-loop scoping is why this needs its own walk
  (`metrics::unbounded_goroutines_in_loops`) rather than reusing the
  `matches_in_loops` single "am I inside a loop" boolean the rest of the
  cluster shares.
- **Naming the launch:** the inline `go func() {...}()` form has no
  callee name, so it surfaces as `func literal` rather than being
  dropped — it's by far the most common shape.
- **New `FindingKind::GoroutineInUnboundedLoop`** (penalty −0.6), the
  heavier tier for the same reason as `defer_in_loop`: one statement
  fires the marker once but spawns a goroutine per iteration.
- **Testing:** unbounded (`WaitGroup`) flagged vs. semaphore-channel
  bounded suppressed; a channel send inside the goroutine doesn't count
  as a bound; a `go` outside any loop is never flagged; a `<-tokens`
  receive at loop-body level does bound the loop; a named launch reports
  its own name. Plus a health-side per-occurrence test and a
  `HealthWeights::default()` assertion.
- **Verified end to end** on a real Go file with both fan-out shapes:
  one hit, on the `WaitGroup` loop.
- **CI note:** the workspace's CI runs clippy with `--all-features`,
  which caught a `too_many_arguments` (8/7) on the new walk's inner
  helper that a plain `--all-targets` run locally did not. Fixed by
  bundling the four classifiers into a small `Scan` struct rather than
  an `#[allow]` — they travel together through both recursions anyway.
- **Scope:** Go only.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR.
- 2 of #72's 19 sub-issues remain open (#182 and #186, both flagged in
  their own issues as needing a design pass this port hasn't taken) —
  this closes only #190, not #72 itself.

---

## PR #226 — Add defer_in_loop health marker (Go)
**2026-07-25** · [#226](https://github.com/baileyrd/rusty_repo_wise/pull/226) · closes [#189](https://github.com/baileyrd/rusty_repo_wise/issues/189)

- **Added:** `defer_in_loop`, the sixteenth slice of issue #72's
  Performance-signal cluster. Flags a Go `defer` statement inside a loop
  body. Go runs deferred calls when the enclosing *function* returns,
  not at the end of the iteration that queued them, so `defer f.Close()`
  in a loop over ten thousand paths holds ten thousand file handles open
  until the whole function exits.
- **Go's first per-language marker classifier.** Go had sat outside this
  entire cluster — no `is_loop` arm, no marker classifiers at all. This
  adds two small ones:
  - `is_loop`: Go has exactly one loop keyword, so the C-style
    three-clause form, the condition-only form, the bare infinite form,
    and `for ... := range ...` are all a single `for_statement` node
    kind. One arm covers all four.
  - `defer_callee`: for a `defer_statement`, the name of the call it
    defers.
- **No callee-name table**, unlike most of this cluster. The `defer`
  keyword *is* the entire signal; the callee name is carried only so the
  finding can say which resource is being held (`Close`, `Unlock`).
- **`Symbol` gains `defer_in_loop: Vec<DeferInLoopRef>`** (each entry:
  `line`, `callee_name`).
- **Shared walk reuse:** `metrics::defers_in_loops` is a thin wrapper
  over the existing `matches_in_loops` family, so nesting depth,
  nested-function skipping, and line attribution behave exactly as they
  do for every other in-loop marker.
- **New `FindingKind::DeferInLoop`** (penalty −0.6). The heavier tier,
  for the same reason as `blocking_io_under_lock`: the cost doesn't land
  on the flagged line. A single `defer` in a loop fires the marker once
  but leaks a resource *per iteration*, so the damage scales with the
  loop's trip count rather than with how often the marker matches —
  counting it linearly like `lock_in_loop` (−0.3) would understate it
  badly.
- **Testing:** an in-loop `defer` is flagged and a function-body `defer`
  is not (the latter is both the correct use and the recommended fix, so
  never penalizing it is the point); all three Go loop forms match at any
  nesting depth; a `defer` inside a `func(){}` literal's loop is
  attributed to the enclosing named function, matching how Go complexity
  already folds function literals in; plus a health-side test for
  one-finding-per-occurrence and a `HealthWeights::default()` assertion.
- **Verified end to end** on a real Go file with one in-loop
  `defer f.Close()` and one correct function-body `defer f.Close()`:
  `repowise health` reports `defer-in-loop 1`.
- **Scope:** Go only, and unavoidably so — no other language in this port
  has a defer-to-function-exit construct, so there is nothing to detect
  elsewhere.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR.
- 3 of #72's 19 sub-issues remain open — this closes only #189, not #72
  itself.

---

## PR #224 — Add sql_cartesian_join health marker
**2026-07-25** · [#224](https://github.com/baileyrd/rusty_repo_wise/pull/224) · closes [#195](https://github.com/baileyrd/rusty_repo_wise/issues/195)

- **Added:** `sql_cartesian_join`, the fifteenth slice of issue #72's
  Performance-signal cluster. Flags a SQL query string listing several
  comma-joined tables with no predicate connecting them — an accidental
  cartesian product returning `n × m` rows. A correctness bug as much as
  a performance one.
- **A text-level marker, not an AST one** — the first in this cluster.
  That turns out to be an advantage: the SQL scan is language-agnostic,
  and each language contributes only a small extractor for its own
  string-literal node kinds. All three converge on a `string_content`/
  `string_fragment` child, so one shared helper shape covers Rust
  (`string_literal`/`raw_string_literal`), Python (`string`), and JS/TS
  (`string`/`template_string`).
- **`Symbol` gains `sql_cartesian_join: Vec<SqlCartesianJoinRef>`** (each
  entry: `line`, `tables`).
- **The heuristic**, deliberately coarse and *not* a SQL parse — the same
  framing as `repowise_workspace::contracts`' route-pattern table: take
  the `FROM` clause up to the next clause keyword, split on commas, and
  require one qualified `a.b = c.d` equality predicate in the `WHERE`
  clause per additional table (`n` tables need `n − 1`).
- **Both sides of a predicate must be qualified.** That's what separates
  a join condition from a plain column filter like `o.status = 1` —
  counting a half-qualified filter would silently suppress a real
  cartesian join. A unit test pins it.
- **Three documented limits:** a `FROM` clause containing an explicit
  `JOIN` is skipped entirely (its `ON` predicate is a different shape,
  and the explicit form is rarely the accidental case); a query
  assembled by string concatenation is invisible, since only one literal
  is ever in hand; and aliases are read as the first whitespace token,
  so unusual formatting can confuse the table list.
- **Testing:** seven unit tests exercise the pure SQL function directly
  — comma-join flagged, proper `WHERE` join accepted, explicit
  `JOIN ... ON` ignored, three tables with one predicate flagged, plain
  column filter not counted, single-table ignored, non-SQL text ignored
  — plus a parser-level test and a health scoring test.
- **New `FindingKind::SqlCartesianJoin`** (penalty −0.6). Arguably the
  most severe marker in the cluster, but also the most heuristic, so it
  shares the quadratic-*shape* tier rather than getting one above it.
- **New dependency:** `regex` added to `repowise-parser`, already in
  `Cargo.lock` via `repowise-workspace` — a zero-new-fetch addition.
- **Scope:** Rust, Python, and TypeScript/JavaScript.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR.
- 4 of #72's 19 sub-issues remain open — this closes only #195, not
  #72 itself.

---

## PR #222 — Add array_spread_in_reduce health marker
**2026-07-25** · [#222](https://github.com/baileyrd/rusty_repo_wise/pull/222) · closes [#194](https://github.com/baileyrd/rusty_repo_wise/issues/194)

- **Added:** `array_spread_in_reduce`, the fourteenth slice of issue
  #72's Performance-signal cluster. Flags a `.reduce(..)`/
  `.reduceRight(..)` callback that builds its result with array spread
  (`(acc, x) => [...acc, x]`) instead of mutating and returning the
  accumulator. The spread copies the *entire* accumulator on every step,
  so a linear fold becomes quadratic — subtle precisely because the
  spread reads as idiomatic immutable-style JS and gives no visual
  signal of the copy.
- **`Symbol` gains `array_spread_in_reduce: Vec<ArraySpreadInReduceRef>`**
  (each entry: `line` of the `reduce` call, `accumulator` parameter
  name).
- **Detection is self-contained in the `reduce` call**, unlike most of
  this cluster — no enclosing loop or function context involved: find a
  `call_expression` whose callee property is `reduce`/`reduceRight`,
  take its first argument as the callback, read that callback's first
  parameter as the accumulator, then check the returned expression is an
  array literal containing a `spread_element` of that same accumulator.
  Both callback body forms are handled (expression-bodied arrow and
  block body with a `return`), and both parameter forms (`(acc, x) =>`
  and single-parameter `acc =>`).
- **New `repowise-parser::metrics::array_spreads_in_reduce`**, built on
  the `matches_in_body` helper added for #184 — no new walking
  machinery.
- **Scope:** **TypeScript/JavaScript only**, per the issue — this
  targets the JS array method, which has no equivalent in this port's
  other languages. The other 14 parsed languages always produce an empty
  list.
- **Two deliberate limits, both erring toward under-reporting:** only
  *top-level* returns in a block body are considered (a `return` nested
  inside an `if` isn't found), and the spread must name the callback's
  own accumulator parameter (spreading an unrelated array isn't
  flagged). Tests pin the non-matches too: the mutate-and-return fix
  (`acc.push(x); return acc`) and a plain scalar fold
  (`(acc, x) => acc + x`) are both left alone.
- **New `FindingKind::ArraySpreadInReduce`** (penalty −0.6, the
  quadratic-*shape* tier alongside `nested_loop_quadratic`/
  `pd_concat_in_loop`).
- **Docs correction:** the README claimed the scorer "covers N of
  repowise's ~25 markers". This marker makes N = 26, which would have
  read "26 of ~25". Reworded to state the real situation instead: this
  port implements 26 distinct markers, exceeding repowise's headline
  figure because that figure counts the Performance-signal work as a
  *single* item while this port implements its pattern checks
  individually (issue #72 alone enumerates 19). The remaining gap is a
  *kind*, not a count — the ML-calibrated organizational-signal markers
  are still deferred.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202/#204/#206/#208/#210/#212/#214/#216/#218/#220).
- 5 of #72's 19 sub-issues remain open — this closes only #194, not
  #72 itself.

---

## PR #220 — Add blocking_io_under_lock health marker
**2026-07-25** · [#220](https://github.com/baileyrd/rusty_repo_wise/pull/220) · closes [#185](https://github.com/baileyrd/rusty_repo_wise/issues/185)

- **Added:** `blocking_io_under_lock`, the thirteenth slice of issue
  #72's Performance-signal cluster. Flags an I/O-shaped call — reusing
  `io_in_loop`'s table, as the issue asked — made while a mutex/lock is
  held. I/O under a lock serializes every *other* thread waiting on that
  lock behind however long the I/O takes.
- **Two lock-scope shapes, not one.** Rust and Python need genuinely
  different extractors:
  - **Rust is structural.** A `let guard = m.lock().unwrap();` binding
    holds the guard until the end of its enclosing block, so every
    statement *after* it is inside the critical section. New
    `repowise-parser::metrics::matches_after_scope_marker` models that:
    the in-scope flag propagates down into children but only turns on
    for *subsequent* siblings, so each node is visited exactly once and
    a nested block with its own guard can't double-report calls the
    outer scope already covered. Reuses `is_lock_call`'s table and so
    inherits its deliberate `RwLock::read`/`write` exclusion.
  - **Python is delimited.** `with lock:` gives an explicit block,
    scanned via the `matches_in_body` helper added for #184.
- **The Python side is a name-based heuristic, deliberately.** Python's
  `with` is generic, and `lock_in_loop` already documents the same
  limitation from the other direction — without type information a lock
  context manager is indistinguishable from a file handle or a database
  transaction. That earlier marker's answer was to skip `with` entirely;
  doing so here would mean dropping Python from a marker whose issue
  names it explicitly. So this matches when the context expression's own
  name looks like a lock (`with lock:`, `with self._write_lock:`,
  `with mutex:`, `with threading.Lock():`). It will miss a lock bound to
  an unconventional name and could in principle fire on a non-lock named
  one; a test pins that a plain `with cm:` stays quiet rather than
  guessing. Rust's side needs no such guess.
- **`Symbol` gains `blocking_io_under_lock: Vec<BlockingIoUnderLockRef>`**
  (each entry: `line`, `callee_name`).
- **New `FindingKind::BlockingIoUnderLock`** (penalty −0.6, matching
  `blocking_sync_in_async` for the same reason: the cost lands on
  threads *other* than the one running the call).
- **Scope:** **Rust and Python only**, per the issue. The other 14
  parsed languages have no lock-scope extractor and always produce an
  empty list.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- **README:** this brings the health scorer to **25 of repowise's ~25
  markers**.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202/#204/#206/#208/#210/#212/#214/#216/#218).
- 6 of #72's 19 sub-issues remain open — this closes only #185, not
  #72 itself.

---

## PR #218 — Add blocking_sync_in_async health marker
**2026-07-25** · [#218](https://github.com/baileyrd/rusty_repo_wise/pull/218) · closes [#184](https://github.com/baileyrd/rusty_repo_wise/issues/184)

- **Added:** `blocking_sync_in_async`, the twelfth slice of issue #72's
  Performance-signal cluster. Flags a known blocking synchronous call
  inside an `async fn`/`async def` body — a blocking call on an async
  executor's worker thread stalls the whole reactor, silently degrading
  every *other* task sharing that thread.
- **A different detection shape.** This is the first marker in the
  cluster whose context is the enclosing **function** rather than an
  enclosing loop, exactly as its issue called out. It shares none of the
  `is_loop` machinery:
  - **New per-language `is_async_fn`** — Rust checks for a
    `function_modifiers` named child containing `async`; Python checks
    for the anonymous `async` token child tree-sitter-python emits on an
    `async def`. Both were verified empirically against the grammars
    before the classifiers were written.
  - **New `repowise-parser::metrics::matches_in_body`** — the non-loop
    counterpart to `matches_in_loops`: same nested-function skipping, no
    loop tracking. `blocking_calls_in_async` wraps it.
  - The extractor only scans a body when `is_async_fn` returns true, so
    non-async functions cost nothing and can never produce entries.
- **`Symbol` gains `blocking_sync_in_async: Vec<BlockingSyncInAsyncRef>`**
  (each entry: `line`, `callee_name`).
- **Scope:** **Rust and Python only**, the two languages the issue scoped
  it to. The other 14 parsed languages have no `is_async_fn` classifier
  and always produce an empty list.
- **Pattern tables**, matched on the qualified two-segment path since a
  bare `sleep`/`read`/`get` would be far too generic:
  - Rust: `thread::sleep`, `fs::read_to_string`, `fs::read`, `fs::write`,
    `fs::copy`, `fs::remove_file`, `fs::create_dir_all`.
  - Python: `time.sleep`, `requests.get`/`post`/`put`/`delete`/`head`/
    `request`, `subprocess.run`/`call`/`check_output`, `os.system`.
  - Python additionally accepts a *bare identifier* callee for `open`,
    distinctive enough as a builtin on its own; a method-form
    `fh.open()` yields the qualified form and never matches that entry.
  - Both tables are deliberately limited to calls with a clear async
    replacement, so every hit has an actionable fix.
- **Correctness detail:** in Rust, `tokio::fs::read_to_string` and
  `std::fs::read_to_string` reduce to the *same* `fs::read_to_string`
  two-segment path, and the tokio one is the async variant that must not
  be flagged. A call that is itself being `.await`ed is therefore never
  reported — being awaited is the only local evidence distinguishing the
  async variant from the blocking one, since this port has no
  import-resolution or type information to consult. Pinned by a test
  with a `tokio::fs::read_to_string(path).await` fixture.
- **New `FindingKind::BlockingSyncInAsync`** (penalty −0.6, elevated
  above the flat −0.3 per-occurrence tier for a *different* reason than
  the quadratic-shape markers: the cost lands on every other task
  sharing the executor, not just on the function containing the call).
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202/#204/#206/#208/#210/#212/#214/#216).
- 7 of #72's 19 sub-issues remain open — this closes only #184, not
  #72 itself.

---

## PR #216 — Add pd_concat_in_loop health marker
**2026-07-25** · [#216](https://github.com/baileyrd/rusty_repo_wise/pull/216) · closes [#192](https://github.com/baileyrd/rusty_repo_wise/issues/192)

- **Added:** `pd_concat_in_loop`, the eleventh slice of issue #72's
  Performance-signal cluster. Flags `pd.concat(..)`/`pandas.concat(..)`
  inside a loop body — the accumulate-one-row-at-a-time shape pandas'
  own docs call out as an anti-pattern. Each call reallocates and copies
  the whole growing DataFrame, making the loop quadratic in the number
  of rows; the fix is to collect rows in a list and concatenate once
  after the loop.
- **`Symbol` gains `pd_concat_in_loop: Vec<PdConcatInLoopRef>`** (each
  entry: `line`, `callee_name`), populated at parse time.
- **New `repowise-parser::metrics::pd_concats_in_loops`**, a thin
  wrapper over the existing `matches_in_loops` shared walk. Python's
  `is_loop` classifier and `qualified_call_name` helper are both reused
  unchanged — only the small name table is new.
- **Scope:** **Python only**, per the issue's own scoping — pandas has
  no equivalent in this port's other supported languages. The other 15
  parsed languages always produce an empty list for this marker; the
  real `Symbol` pushes in `rust.rs`/`javascript.rs` carry an explanatory
  comment, matching the `list_insert_zero_in_loop` precedent.
- **Deliberate deviation from the issue's wording:** the acceptance
  criteria named a table of `pd.concat`/`pandas.concat`/DataFrame
  `.append`. This ships the first two and **deliberately excludes bare
  `.append`**. Without type information this port cannot distinguish
  `DataFrame.append` from `list.append` — and appending to a *list*
  inside a loop is precisely the fix this marker recommends, so flagging
  bare `.append` would fire on the correct pattern far more often than
  the wrong one, pushing users away from the fix. (`DataFrame.append`
  was also deprecated in pandas 1.4 and removed in 2.0, so its
  real-world incidence is shrinking regardless.) Documented in code and
  README, and pinned by a test: the fixture's recommended-fix function
  does `parts.append(r)` inside a loop and asserts it is *not* flagged.
- **New `FindingKind::PdConcatInLoop`** (penalty −0.6 — the
  quadratic-*shape* tier alongside `nested_loop_with_io`/
  `nested_loop_quadratic`, deliberately not the flat −0.3
  per-occurrence tier, since one such call is an order-of-magnitude
  blowup rather than one extra unit of work).
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202/#204/#206/#208/#210/#212/#214).
- 8 of #72's 19 sub-issues remain open — this closes only #192, not
  #72 itself.

---

## PR #214 — Add serial_await_in_loop health marker
**2026-07-25** · [#214](https://github.com/baileyrd/rusty_repo_wise/pull/214) · closes [#181](https://github.com/baileyrd/rusty_repo_wise/issues/181)

- **Added:** `serial_await_in_loop`, the tenth slice of issue #72's
  Performance-signal cluster. Flags an awaited async call inside a loop
  body — each iteration blocks on the previous one, turning what could
  be one concurrent batch into N sequential round-trips.
- **`Symbol` gains `serial_await_in_loop: Vec<SerialAwaitInLoopRef>`**
  (each entry: `line`, `callee_name`), populated at parse time.
- **New `repowise-parser::metrics::serial_awaits_in_loops`**, a thin
  wrapper over the existing `matches_in_loops` shared walk — no new
  walking machinery this time, only a new per-language classifier.
- **Scope:** **Rust, Python, and TypeScript/JavaScript** — the three
  parsed languages whose grammars carry async syntax, which the issue
  explicitly asked be confirmed before committing to a language list.
  Verified empirically against each grammar: `await_expression` in Rust
  and TS/JS, `await` in Python. The other 13 parsed languages get an
  empty `serial_await_in_loop` list.
- **Two deliberate narrowings**, both covered by a test per language:
  - *Only awaits whose operand is a call are flagged.* An await on an
    already-created future (`await somePromise`) isn't reported — the
    issue describes "each iteration's async call", and requiring a call
    is also what lets every finding name what's being awaited.
  - *Awaits of concurrency combinators are excluded* —
    `Promise.all`/`allSettled`/`race`/`any` (TS/JS),
    `join_all`/`try_join_all`/`join`/`try_join`/`select_all` (Rust),
    `gather`/`as_completed` (Python). Those are precisely the *fix* this
    marker recommends, so awaiting one inside a loop is the deliberate
    chunked-concurrency shape (`for chunk in chunks { await
    Promise.all(chunk.map(..)) }`), not the serial one — flagging it
    would punish the correct pattern.
- **Matching precision:** TS/JS matches the combinator on its qualified
  `Promise.all` form, since a bare `all`/`race` would be far too
  generic. Python matches bare `gather`/`as_completed` (distinctive on
  their own, and robust to `from asyncio import gather`) but
  deliberately omits `asyncio.wait` — a bare `wait` is not distinctive,
  and omitting it costs at most a false positive, never a false
  negative.
- **New `FindingKind::SerialAwaitInLoop`** (penalty −0.3 — the
  per-occurrence loop-body tier, deliberately *not* the −0.6
  quadratic-*shape* tier used by `nested_loop_with_io`/
  `nested_loop_quadratic`: one serialized await is one extra round-trip,
  not an order-of-magnitude blowup).
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202/#204/#206/#208/#210/#212).
- 9 of #72's 19 sub-issues remain open — this closes only #181, not
  #72 itself.

---

## PR #212 — Add nested_loop_quadratic health marker
**2026-07-25** · [#212](https://github.com/baileyrd/rusty_repo_wise/pull/212) · closes [#187](https://github.com/baileyrd/rusty_repo_wise/issues/187)

- **Added:** `nested_loop_quadratic`, the ninth slice of issue #72's
  Performance-signal cluster. Flags an inner loop iterating the *same
  collection* as an enclosing loop — the classic accidental all-pairs
  O(n²) scan (`for x in items { for y in items { .. } }`), usually
  replaceable with a set/map lookup.
- **`Symbol` gains `nested_loop_quadratic: Vec<NestedLoopQuadraticRef>`**
  (each entry: `line` of the inner loop, `iterable` shared collection
  name), populated at parse time.
- **New `repowise-parser::metrics::quadratic_loop_nestings`** — walks
  carrying a *stack* of the enclosing loops' normalized iterable names,
  reporting any loop whose own name is already on that stack. Takes no
  separate `is_loop` predicate (unlike its sibling walks): `loop_iterable`
  already node-kind-checks for its language's `for`-loop form, and only a
  `for` loop has an iterable to compare, so a second classifier would be
  redundant and could silently disagree.
- **Per-language `loop_iterable` normalizers** reduce an iterable
  expression to a base collection name, peeling only wrappers yielding
  the *same* underlying collection:
  - Rust: `items`, `&items`, `items.iter()`/`.iter_mut()`/`.into_iter()`.
  - Python: `items`, `enumerate(items)`/`sorted(items)`/`reversed(items)`,
    `d.values()`/`.keys()`/`.items()`.
  - TypeScript/JavaScript: `items`, `items.values()`/`.keys()`/
    `.entries()`/`.slice()`, and `Object.keys(items)`.
  A genuinely narrower sequence (`items.filter(..)`, a comprehension)
  doesn't normalize and so never compares equal to anything.
- **Correctness detail:** JS's `Object.keys(x)` resolves to `x`, not the
  `Object` global. Peeling to the receiver like the other `.keys()` forms
  would collapse every such loop to the name `Object`, making two
  unrelated collections falsely match. Handled explicitly and covered by
  a test.
- **Relationship to `nested_loop_with_io` (#183):** complementary, not
  overlapping. That marker compares nesting *depth* and inspects the
  body; this one compares the loops' *iterable expressions* and ignores
  the body. A cross-product over two different collections isn't flagged
  here; an all-pairs scan doing no I/O isn't flagged there.
- **Deliberate exclusion:** ranges (`for i in 0..n`, `range(n)`, C-style
  `for (let i = 0; ...)`) don't normalize, so a doubly-nested index walk
  isn't flagged — that shape is usually a deliberate, irreducible
  grid/matrix traversal rather than the accidental one-collection scan
  this marker targets, matching the issue's own "same collection
  variable" wording.
- **New `FindingKind::NestedLoopQuadratic`** (penalty −0.6, matching
  `nested_loop_with_io` — both flag a quadratic-complexity *shape*
  rather than a single expensive call, a tier above the flat −0.3
  per-occurrence loop-body markers).
- **Scope:** **Rust, Python, and TypeScript/JavaScript**, matching
  `io_in_loop`'s scope. The other 13 parsed languages get an empty
  `nested_loop_quadratic` list.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202/#204/#206/#208/#210).
- 10 of #72's 19 sub-issues remain open — this closes only #187, not
  #72 itself.

---

## PR #210 — Add nested_loop_with_io health marker
**2026-07-24** · [#210](https://github.com/baileyrd/rusty_repo_wise/pull/210) · closes [#183](https://github.com/baileyrd/rusty_repo_wise/issues/183)

- **Added:** `nested_loop_with_io`, the eighth slice of issue #72's
  Performance-signal cluster. Flags a known I/O-shaped call found at
  loop-nesting depth 2 or deeper — potentially O(n²) or worse I/O calls,
  rather than the O(n) a single-loop `io_in_loop` hit represents.
- **`Symbol` gains `nested_loop_with_io: Vec<NestedLoopWithIoRef>`** (each
  entry: `line`, `callee_name`), populated at parse time.
- **New `repowise-parser::metrics::matches_in_nested_loops`** — the first
  structural addition to this cluster's shared machinery since
  `matches_in_loops` itself. Structurally parallel to it, but tracks a
  running loop-nesting *depth* instead of a single in-loop boolean, and
  only reports a match once depth reaches a caller-supplied minimum.
  Needed because this is the first marker that distinguishes "inside one
  loop" from "inside a loop nested inside another loop"; the other seven
  loop-body markers only care whether a loop encloses the call at all.
- **New `repowise-parser::metrics::ios_in_nested_loops`** wraps that walk
  with `min_depth = 2`, the same way `calls_in_loops` wraps the plain
  version.
- **Scope:** implemented for **Rust, Python, and TypeScript/JavaScript**,
  matching `io_in_loop`'s scope. No new per-language pattern tables were
  needed — each language's existing `is_loop`/`call_expression_callee`/
  `is_io_call` are reused unchanged, only plumbed through the new walk.
  The other 13 parsed languages get an empty `nested_loop_with_io` list.
- **New `FindingKind::NestedLoopWithIo`** (penalty **−0.6**, double every
  other loop-body marker's flat −0.3), one `Finding` per flagged call,
  pointing at the call's own line.
- **Deliberate double-counting:** a call flagged here is *also* flagged
  under `io_in_loop` — this is a depth-2+ subset of that marker, not a
  separate detection pass. The outer marker measures "any loop-body
  I/O"; this one measures the specifically worse nested case, and the
  heavier penalty reflects that.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202/#204/#206/#208).
- 11 of #72's 19 sub-issues remain open — this closes only #183, not
  #72 itself.

---

## PR #208 — Add regex_compile_in_loop health marker
**2026-07-24** · [#208](https://github.com/baileyrd/rusty_repo_wise/pull/208) · closes [#188](https://github.com/baileyrd/rusty_repo_wise/issues/188)

- **Added:** `regex_compile_in_loop`, the seventh slice of issue #72's
  Performance-signal cluster. Flags a known regex-compilation call
  (`Regex::new`, `re.compile`, `new RegExp`) found inside a loop body —
  compiling a regex is orders of magnitude more expensive than using an
  already-compiled one, so doing it once per loop iteration instead of
  once before the loop is a common, easily-fixed performance mistake.
- **`Symbol` gains `regex_compile_in_loop: Vec<RegexCompileInLoopRef>`**
  (each entry: `line`, `callee_name`), populated at parse time.
- **New `repowise-parser::metrics::regex_compiles_in_loops`**, a thin
  wrapper on the shared `matches_in_loops` walk — no new walking logic
  needed, same shape as the five other loop-body markers already built
  on it.
- **Scope:** implemented for **Rust, Python, and TypeScript/JavaScript**,
  matching `io_in_loop`'s scope.
  - Rust: reuses `qualified_call_name` (already extracting `Type::method`
    qualified paths for `resource_construction_in_loop`/
    `json_parse_in_loop`) for `Regex::new`'s qualified form — a bare
    `new` alone would match `Vec::new()`.
  - Python: reuses `qualified_call_name` (`object.attribute`, e.g.
    `re.compile`) — a bare `compile` alone is too generic (`ast.compile`,
    other `.compile()` methods).
  - TypeScript/JavaScript: reuses `resource_constructor_callee` (handles
    both `call_expression` and `new_expression`) with a new
    `is_regex_compile_call` table matching the bare `RegExp` constructor
    name — unlike Rust/Python, `RegExp` is already distinctive enough on
    its own, no qualified form needed.
  - The other 13 parsed languages get an empty `regex_compile_in_loop`
    list and never trigger this marker.
- **New `FindingKind::RegexCompileInLoop`** (penalty −0.3, same weight as
  the other loop-body markers), one `Finding` per flagged call, pointing
  at the call's own line.
- This is the marker `resource_construction_in_loop`'s own table already
  excluded `Regex::new`/`re.compile`/`new RegExp` for (issue #179), so
  the two markers don't double-flag the same call now that both exist.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202/#204/#206).
- 12 of #72's 19 sub-issues remain open — this closes only #188, not
  #72 itself.

---

## PR #206 — Add json_parse_in_loop health marker
**2026-07-24** · [#206](https://github.com/baileyrd/rusty_repo_wise/pull/206) · closes [#193](https://github.com/baileyrd/rusty_repo_wise/issues/193)

- **Added:** `json_parse_in_loop`, the sixth slice of issue #72's
  Performance-signal cluster. Flags a known JSON-deserializing call
  (`serde_json::from_str`/`from_slice`, `json.loads`/`json.load`,
  `JSON.parse`) found inside a loop body — parsing the same/similarly-
  shaped payload once per iteration is usually avoidable by parsing once
  outside the loop, or restructuring to parse a single batched payload.
- **`Symbol` gains `json_parse_in_loop: Vec<JsonParseInLoopRef>`** (each
  entry: `line`, `callee_name`), populated at parse time.
- **New `repowise-parser::metrics::json_parses_in_loops`**, a thin
  wrapper on the shared `matches_in_loops` walk — no new walking logic
  needed, same shape as `calls_in_loops`/`locks_in_loops`/
  `resource_constructions_in_loops`.
- **Scope:** implemented for **Rust, Python, and TypeScript/JavaScript**,
  matching `io_in_loop`'s scope.
  - Rust: reuses (and renames) `qualified_constructor_name` →
    `qualified_call_name` — already extracting `Type::method` qualified
    paths for `resource_construction_in_loop`, now reused for
    `serde_json::from_str`/`from_slice`'s `module::function` shape too.
  - Python: new `qualified_call_name` helper (`object.attribute`, e.g.
    `json.loads`) — a bare `loads`/`load` would be dangerously generic
    (`pickle.load`, `yaml.load`, any other `.load()` method).
  - TypeScript/JavaScript: new `qualified_call_name` helper
    (`object.property`, e.g. `JSON.parse`) — same generic-bare-name
    problem (`Date.parse`, other `.parse()` methods).
  - The other 13 parsed languages get an empty `json_parse_in_loop` list
    and never trigger this marker.
- **New `FindingKind::JsonParseInLoop`** (penalty −0.3, same weight as
  the other loop-body markers), one `Finding` per flagged call, pointing
  at the call's own line.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202/#204).
- 13 of #72's 19 sub-issues remain open — this closes only #193, not
  #72 itself.

---

## PR #204 — Add list_insert_zero_in_loop health marker
**2026-07-24** · [#204](https://github.com/baileyrd/rusty_repo_wise/pull/204) · closes [#191](https://github.com/baileyrd/rusty_repo_wise/issues/191)

- **Added:** `list_insert_zero_in_loop`, the fifth slice of issue #72's
  Performance-signal cluster. Flags `.insert(0, ...)` on a list/vector
  found inside a loop body — O(n) per call (shifts every element), O(n²)
  across the whole loop, versus appending and reversing once or using a
  deque.
- **`Symbol` gains `list_insert_zero_in_loop: Vec<ListInsertZeroInLoopRef>`**
  (each entry: `line`, `variable`), populated at parse time.
- **New `repowise-parser::metrics::list_inserts_zero_in_loops`**, built
  on the shared `matches_in_loops` walk. Unlike `calls_in_loops`/
  `locks_in_loops`/`resource_constructions_in_loops` (which filter a
  plain callee name against a fixed table), this classifier needs to
  inspect the call's *arguments* too (the first argument must be the
  literal `0`), so each language gets a single combined classifier
  rather than a name-table filter step.
- **Scope:** implemented for **Rust and Python only**, per this issue's
  own acceptance criteria — unlike the other four Performance-signal
  loop-body markers (#177/#178/#179/#180), this one's scope doesn't
  extend to TypeScript/JavaScript.
  - Rust: `.insert(0, ...)` on an identifier receiver, covering both
    `Vec::insert`/`VecDeque::insert` since this port has no type
    information to distinguish the two.
  - Python: `list.insert(0, ...)` the same way.
  - The other 14 parsed languages get an empty
    `list_insert_zero_in_loop` list and never trigger this marker.
- **New `FindingKind::ListInsertZeroInLoop`** (penalty −0.3, same weight
  as the other loop-body markers), one `Finding` per flagged insert,
  pointing at the insert's own line.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR
  (#127/#129/#196/#198/#200/#202).
- 14 of #72's 19 sub-issues remain open — this closes only #191, not
  #72 itself.

---

## PR #202 — Add lock_in_loop health marker
**2026-07-24** · [#202](https://github.com/baileyrd/rusty_repo_wise/pull/202) · closes [#180](https://github.com/baileyrd/rusty_repo_wise/issues/180)

- **Added:** `lock_in_loop`, the fourth slice of issue #72's
  Performance-signal cluster. Flags a mutex/lock acquisition call
  happening inside a loop body — repeated lock/unlock churn per
  iteration instead of acquiring the lock once outside the loop.
- **`Symbol` gains `lock_in_loop: Vec<LockInLoopRef>`** (each entry:
  `line`, `callee_name`), populated at parse time.
- **New `repowise-parser::metrics::locks_in_loops`**, built on the
  shared `matches_in_loops` walk introduced in PR #200 — same shape as
  `calls_in_loops`/`resource_constructions_in_loops`.
- **Scope:** implemented for **Rust, Python, and TypeScript/JavaScript
  only**, matching the other three loop-body markers' precedent.
  - Rust: `.lock()`/`.try_lock()` — deliberately excludes
    `RwLock::read`/`write`, since those bare method names are far too
    generic on their own (shared with `Read`/`Write` trait methods,
    plain getters/setters) and this port has no type information to
    know a receiver is actually an `RwLock`.
  - Python: `.acquire()` (`threading.Lock`/`RLock`) — the `with lock:`
    shape isn't recognized, since distinguishing a lock context manager
    from any other `with` statement would need type information this
    port doesn't have.
  - TypeScript/JavaScript: `.acquire()`, mirroring common userland lock
    libraries (e.g. `async-mutex`) since JS has no native mutex.
  - The other 13 parsed languages get an empty `lock_in_loop` list and
    never trigger this marker.
- **New `FindingKind::LockInLoop`** (penalty −0.3, same weight as the
  other loop-body markers), one `Finding` per flagged acquisition,
  pointing at the acquisition's own line.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR (#127/#129/#196/#198/#200).
- 15 of #72's 19 sub-issues remain open — this closes only #180, not
  #72 itself.

---

## PR #200 — Add resource_construction_in_loop health marker
**2026-07-24** · [#200](https://github.com/baileyrd/rusty_repo_wise/pull/200) · closes [#179](https://github.com/baileyrd/rusty_repo_wise/issues/179)

- **Added:** `resource_construction_in_loop`, the third slice of issue
  #72's Performance-signal cluster. Flags construction of a known
  expensive resource (an HTTP client, a connection/thread pool) found
  inside a loop body, where hoisting the construction above the loop is
  usually possible.
- **`Symbol` gains `resource_construction_in_loop: Vec<ResourceConstructionInLoopRef>`**
  (each entry: `line`, `callee_name`), populated at parse time.
- **Refactor:** extracted a shared generic `repowise-parser::metrics::matches_in_loops<T>`
  tree-walk out of `calls_in_loops`/`string_concats_in_loops` — with this
  marker, the third near-identical "X found inside a loop" shape,
  de-duplicating the walk logic was worth doing now rather than
  copy-pasting a fourth near-identical function. Both existing functions
  keep their exact public signature and return type; only their
  internals now delegate to the shared walk, so no call sites in
  `rust.rs`/`python.rs`/`javascript.rs` needed to change for #177/#178.
- **New `repowise-parser::metrics::resource_constructions_in_loops`**
  built on top of that shared walk.
- **Scope:** implemented for **Rust, Python, and TypeScript/JavaScript
  only**, matching `io_in_loop`/`string_concat_in_loop`'s precedent.
  Rust matches on the *qualified* `Type::method` path (e.g.
  `HttpClient::new`) rather than a bare method name, since `new` alone
  would match `Vec::new()`/`String::new()`; Python/JS match on the bare
  constructor/class name. Deliberately excludes cheap allocation
  constructors (`Vec::with_capacity`, `String::new`) per the issue's own
  acceptance criteria, and excludes regex construction (`Regex::new`/
  `re.compile`/`new RegExp`) — reserved for `regex_compile_in_loop`
  (issue #188) so the two markers don't double-flag the same call once
  both exist. The other 13 parsed languages get an empty
  `resource_construction_in_loop` list and never trigger this marker.
- **New `FindingKind::ResourceConstructionInLoop`** (penalty −0.3, same
  weight as `IoInLoop`/`StringConcatInLoop`), one `Finding` per flagged
  construction, pointing at the construction's own line.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR (#127/#129/#196/#198).
- 16 of #72's 19 sub-issues remain open — this closes only #179, not
  #72 itself.

---

## PR #198 — Add string_concat_in_loop health marker
**2026-07-24** · [#198](https://github.com/baileyrd/rusty_repo_wise/pull/198) · closes [#178](https://github.com/baileyrd/rusty_repo_wise/issues/178)

- **Added:** `string_concat_in_loop`, the second slice of issue #72's
  Performance-signal cluster. Flags a string-append expression (`+=`,
  `s = s + other`, `.push_str(..)`) accumulating onto a variable inside
  a loop body — quadratic string-building cost, since each append
  reallocates and copies the whole string built so far.
- **`Symbol` gains `string_concat_in_loop: Vec<StringConcatInLoopRef>`**
  (each entry: `line`, `variable`), populated at parse time.
- **New `repowise-parser::metrics::string_concats_in_loops`**, reusing
  `io_in_loop`'s (#177) `is_loop` classifier and mirroring
  `calls_in_loops`'s "currently inside a loop" tracking shape exactly —
  just matching a different per-language classifier (a string-append
  expression instead of an I/O-shaped call).
- **Scope:** implemented for **Rust, Python, and TypeScript/JavaScript
  only**, matching `io_in_loop`/LCOM4/`complex_conditional`'s precedent.
  Each language recognizes a compound `+=` assignment onto a bare
  identifier, and a `s = s + other` reassignment (an assignment whose
  right side is a `+` binary expression naming the left-hand identifier
  on either side); Rust additionally recognizes `s.push_str(other)`,
  since Python/JS strings are immutable and have no equivalent mutating
  method. The other 13 parsed languages get an empty
  `string_concat_in_loop` list and never trigger this marker.
- **New `FindingKind::StringConcatInLoop`** (penalty −0.3, same weight
  as `IoInLoop`), one `Finding` per flagged append, pointing at the
  append's own line.
- **Incidental fix:** boxed `repowise-graph::Node`'s `Symbol` variant
  (`Symbol(Symbol)` → `Symbol(Box<Symbol>)`) — `Symbol` growing another
  `Vec` field tripped `clippy::large_enum_variant`. `Node` is a private
  implementation detail used only inside `repowise-graph`, so this only
  touched 4 call sites, all in that one file.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR (#127/#129/#196).
- 17 of #72's 19 sub-issues remain open — this closes only #178, not
  #72 itself.

---

## PR #196 — Add io_in_loop health marker
**2026-07-24** · [#196](https://github.com/baileyrd/rusty_repo_wise/pull/196) · closes [#177](https://github.com/baileyrd/rusty_repo_wise/issues/177)

- **Added:** `io_in_loop`, the first slice of issue #72's Performance-signal
  cluster (~19 pattern checks tracked as sub-issues #177-#195). Flags a
  known file/network/database call found anywhere inside a loop body,
  where hoisting the call above the loop is usually possible.
- **`Symbol` gains `io_in_loop: Vec<IoInLoopRef>`** (each entry: `line`,
  `callee_name`), populated at parse time.
- **New `repowise-parser::metrics::calls_in_loops`**, mirroring
  `complex_conditionals`'s shape but tracking a single "currently inside
  a loop" flag down the whole body walk, so a call nested inside two
  loops is still only reported once, at its own line, rather than once
  per enclosing loop. Driven by two per-language pieces: an `is_loop`
  classifier (a subset of each language's existing `is_decision` node
  kinds — loops only, not `if`/`match`/etc., which branch but don't
  repeat) and a small fixed table of I/O-shaped callee names.
- **Scope:** implemented for **Rust, Python, and TypeScript/JavaScript
  only** for this first pass, matching the LCOM4/`complex_conditional`
  precedent (issues #51/#55). The other 13 parsed languages get an
  empty `io_in_loop` list and never trigger this marker.
- Like `unresolved_import_stems`/`repowise-workspace::contracts`'s
  route-matching table, the I/O-callee-name table is deliberately coarse
  and heuristic: matching on a call's last name/segment means it can't
  tell a database cursor's `.execute(..)` from an unrelated `execute`
  method on some other type, and it can't recognize I/O hidden behind a
  wrapper function the table doesn't name.
- **New `FindingKind::IoInLoop`** (penalty −0.3, same weight as
  `ComplexConditional`), one `Finding` per flagged call, pointing at the
  call's own line.
- **Mechanical fallout:** `Symbol`'s new field touched its construction
  site in all 16 language parsers plus test fixtures across
  `repowise-adr`/`repowise-dashboard`/`repowise-docs`/`repowise-git`/
  `repowise-health`/`repowise-server` that build `Symbol` directly.
- Pre-existing `.repowise/index.json` files need a re-`init`/`update` —
  same as every prior `Symbol`-field-adding PR (#127/#129): the field
  has no `#[serde(default)]`, so an old index fails to deserialize
  until regenerated. `index.json` is a regenerable cache, not a durable
  format, so this isn't tracked as a breaking change.
- 18 of #72's 19 sub-issues remain open (`string_concat_in_loop`,
  `resource_construction_in_loop`, `lock_in_loop`,
  `serial_await_in_loop`, `membership_test_against_list_in_loop`,
  `nested_loop_with_io`, `blocking_sync_in_async`,
  `blocking_io_under_lock`, `hot_path_sync_io`, `nested_loop_quadratic`,
  `regex_compile_in_loop`, `defer_in_loop`,
  `goroutine_in_unbounded_loop`, `list_insert_zero_in_loop`,
  `pd_concat_in_loop`, `json_parse_in_loop`, `array_spread_in_reduce`,
  `sql_cartesian_join`) — this closes only #177, not #72 itself.

---

## PR #175 — Add Structural-tier language recognition
**2026-07-24** · [#175](https://github.com/baileyrd/rusty_repo_wise/pull/175) · closes [#70](https://github.com/baileyrd/rusty_repo_wise/issues/70)

- **Added:** issue #70 asked to verify whether `hotspots`/`ownership`/
  `coupled` already work for Objective-C, R, Zig, Julia, Elm, OCaml,
  Crystal, Nim, and D files (previously `Language::Other`), since git
  analytics is file-path based, not symbol based. Investigation found a
  real, partial gap: `ownership`/`coupled` already worked (both take an
  explicit file path and read straight from `git blame`/`git log`,
  bypassing `RepoIndex` entirely) — but `hotspots` and any other
  `RepoIndex.files`-driven view never showed these files at all, because
  a `Language::Other` file gets **no `FileRecord`**, just folded into a
  bare `other_files: usize` count with no path retained anywhere.
- These 9 languages are now recognized `Language` variants with a bare,
  zero-symbol `FileRecord` (path/language/line count only, via a new
  `repowise_parser::structural_only` helper) instead of falling into
  `other_files`. Their hotspot score is always `0` (churn × 0
  complexity, since no tree-sitter grammar exists for them) — git-history
  signal only, matching repowise's own "Structural tier" framing.
- `repowise overview` now reports real per-language file counts for
  these languages instead of lumping them into "Other"; `repowise
  hotspots` now lists them (with a `0` score); the dashboard's
  dependency-graph view gained GitHub-linguist colors for all 9.
- No breaking changes — purely additive (new enum variants, new
  dispatch arms). Existing indexes still load fine; a re-`init`/`update`
  is needed for these languages' files to start appearing.
- **This closes #70.**

---

## PR #173 — Add contracts view (producer/consumer API matching)
**2026-07-24** · [#173](https://github.com/baileyrd/rusty_repo_wise/pull/173) · closes [#64](https://github.com/baileyrd/rusty_repo_wise/issues/64)

- **Added:** the fifth and final slice of issue #64. Producer/consumer
  API contract matching — fully independent of the other four #64
  slices, with no cross-repo symbol resolution involved at all. A
  regex-based scan of each indexed file's raw text for a small, fixed
  table of HTTP route-registration patterns (axum `.route("/path",
  get(...))`, Flask/FastAPI `@app.get("/path")`, Express
  `app.get("/path", ...)`) and HTTP-call patterns (JS
  `fetch`/`axios.get`, Python `requests.get`, Rust `ureq::get`),
  matching each consumer call against producer routes registered in
  *other* repos (segment-wise, treating a producer path segment like
  `:id`/`{id}` as a wildcard).
- New `repowise-workspace::contracts` module
  (`workspace_contracts`/`ProducerRoute`/`ConsumerCall`/
  `ContractMatch`/`ContractsReport`), a new `GET
  /api/workspace-contracts` endpoint, a new **Contracts** dashboard
  section (matched pairs + unmatched consumer calls as two separate
  lists), and a new `repowise workspace-contracts --workspace <path>`
  CLI subcommand.
- Uses `regex = "1"`, already fully resolved in `Cargo.lock` via
  `tree-sitter` (a `repowise-parser` dependency) — no new crate version
  added to the dependency tree.
- **Coarse and heuristic by design**, the same honesty this port
  already applies to `unresolved_import_stems`/`repowise-adr`'s
  keyword-based commit mining: a real implementation would need to
  parse each web framework's actual route-registration semantics per
  language, which this port has no such capability for. False
  negatives (an unrecognized framework idiom) and false positives (a
  route-shaped string that isn't actually a route) are both expected.
  An "unmatched consumer" finding is not necessarily a problem — it may
  be a call to a genuinely external API.
- No breaking changes — purely additive (new crate module, new route,
  new dashboard section, new CLI subcommand).
- **This closes #64.** All five originally bundled items are now
  shipped: `list_repos` (PR #165), workspace co-change reporting (PR
  #167), `get_architecture`/`get_blast_radius`/System Map (PR #169),
  conformance (PR #171), and contracts (this PR).

---

## PR #171 — Add conformance view (circular cross-repo dependencies)
**2026-07-24** · [#171](https://github.com/baileyrd/rusty_repo_wise/pull/171) · part of [#64](https://github.com/baileyrd/rusty_repo_wise/issues/64)

- **Added:** the fourth slice of issue #64. Circular cross-repo
  dependency detection (repo A imports repo B imports repo A, or a
  longer chain), reusing exactly the edges PR #169's
  `workspace_architecture` already computes — no new resolution logic,
  just `repowise_graph::detect_repo_cycles` (petgraph `kosaraju_scc`
  over repo-level edges, added but unused in #169) wired up to a real
  surface. A workspace's repo-level dependency graph should form a DAG;
  a cycle is a concrete, deterministic "pattern divergence" finding
  needing no further human-specified rule set to detect.
- New `GET /api/workspace-conformance` endpoint (`{"available": bool,
  "cycles": [[repo names]]}`) and a **Conformance** dashboard section.
- New `repowise workspace-conformance --workspace <path>` CLI
  subcommand.
- No new MCP tool — the tracking issue only names
  `get_architecture`/`get_blast_radius` as MCP tools; conformance is
  dashboard-only.
- No breaking changes — purely additive (new route, new dashboard
  section, new CLI subcommand).
- **This does not close #64** — the contracts (producer/consumer API
  matching) dashboard view remains, the last of the five bundled items.

---

## PR #169 — Add cross-repo Rust import resolution
**2026-07-24** · [#169](https://github.com/baileyrd/rusty_repo_wise/pull/169) · part of [#64](https://github.com/baileyrd/rusty_repo_wise/issues/64)

- **Added:** the third slice of issue #64 — real cross-repo dependency
  resolution. `repowise-graph` gained `cross_repo_import_edges`: an
  unresolved Rust `use` import in one workspace repo is now resolved
  against another repo's Rust module-path map (`crate::path` -> file,
  derived from each repo's own `Cargo.toml`), rather than left
  permanently unresolved. An import counts as a cross-repo candidate
  only if it's unresolved both at parse time AND against its **own**
  repo's module map — a sibling-crate import within one multi-crate
  repo (this port's own layout, for example) is never mistaken for a
  cross-repo edge just because another repo happens to define a
  same-named module.
- Two new MCP tools: `get_architecture` (workspace-wide repo-pair
  dependency summary + individual import sites; degrades to empty
  lists like `list_repos` when no `--workspace` was given) and
  `get_blast_radius` (direct, one-hop cross-repo importers of one
  file — matches `RepoGraph::dependents_of`'s existing single-repo
  precedent, which is also direct-only, not transitive; errors like
  `get_context` on an unknown repo or unindexed file, since it targets
  one specific file rather than degrading).
- New `GET /api/workspace-architecture` endpoint and a **System Map**
  dashboard section — a plain repo-pair table with individual import
  sites listed underneath, not a force-directed graph, since repo-level
  granularity is small.
- New CLI subcommands: `repowise workspace-architecture --workspace
  <path>` and `repowise workspace-blast-radius --workspace <path>
  --repo <name> --file <path>`.
- Rust-only for now — the only language this port anchors to a
  `Cargo.toml`-derived crate name; every other language's cross-repo
  imports are left unresolved, deliberately, for a future slice.
- Also added `repowise_graph::detect_repo_cycles` and a
  `repowise_workspace::detect_workspace_cycles` wrapper (petgraph
  `kosaraju_scc` over repo-level edges) — unused by any surface in this
  PR, laying groundwork for the next #64 slice (circular cross-repo
  dependency detection powering a conformance view) without adding new
  surface area here.
- No breaking changes — purely additive (new crate functions, new MCP
  tools, new route, new dashboard section, new CLI subcommands).
- **This does not close #64** — the conformance (pattern divergence)
  and contracts (producer/consumer API matching) dashboard views remain.

---

## PR #167 — Add workspace co-change reporting
**2026-07-24** · [#167](https://github.com/baileyrd/rusty_repo_wise/pull/167) · part of [#64](https://github.com/baileyrd/rusty_repo_wise/issues/64)

- **Added:** the second slice of issue #64. Each workspace repo's own
  most-coupled file pairs (from `repowise-git`'s existing
  `GitAnalytics`), shown side by side. This is **not** cross-repo
  co-change — separate repos have separate git histories, so files in
  different repos can never literally co-change in the same commit —
  just each repo's own coupling rendered together in one place, with no
  new cross-repo dependency resolution required.
- `repowise-git` gained `GitAnalytics::top_co_changed_pairs(top_n)`,
  ranking every co-changed pair across a repo's whole history (the
  existing `coupled_files` was scoped to one file at a time).
  `repowise-workspace` gained `workspace_co_changes(repos, top_n)` +
  `RepoCoChanges`/`CoChangePair`, degrading to `available: false` for a
  repo with no readable git history — same shape as `RepoStatus`.
- Two new ways to see it: `repowise workspace-co-changes --workspace
  <path> [--top <N>]` (CLI), and `GET /api/workspace-co-changes` plus a
  new Workspace Co-Changes dashboard section (opt-in via `repowise
  serve-dashboard --workspace <path>`).
- `repowise-workspace` now also depends on `repowise-git` (previously
  only `repowise-core`) — still deliberately its own crate, not folded
  into `repowise-core`, so a future cross-repo slice can add a
  `repowise-graph` dependency too without `repowise-core` ever
  depending upward.
- **Still deliberately excluded:** `get_architecture`/`get_blast_radius`
  (MCP tools) and the dashboard's `/workspace/system-map`,
  `/workspace/conformance`, `/workspace/contracts` views all need real
  cross-repo dependency resolution, which doesn't exist anywhere in this
  port yet — left for a follow-up.
- No breaking changes — purely additive (new crate function, new CLI
  subcommand, new route, new dashboard section).
- **This does not close #64** — cross-repo tools and views remain.

---

## PR #165 — Add multi-repo workspace repo listing
**2026-07-24** · [#165](https://github.com/baileyrd/rusty_repo_wise/pull/165) · part of [#64](https://github.com/baileyrd/rusty_repo_wise/issues/64)

- **Added:** the first slice of issue #64 (multi-repo/workspace
  support). A new `repowise-workspace` crate parses a small standalone
  TOML file naming a set of repo roots (never inferred from or stored
  inside any member repo's own `.repowise/` — a workspace spans repos,
  so no single member repo is a sensible owner of it), and reports each
  configured repo's indexed status.
- Three new ways to list a workspace's repos: `repowise workspace-repos
  --workspace <path>` (CLI), a new `list_repos` MCP tool (opt-in via
  `repowise serve --workspace <path>`), and `GET /api/workspace-repos`
  plus a new Workspace repo-cards section on the dashboard (opt-in via
  `repowise serve-dashboard --workspace <path>`).
- **Deliberately excluded:** `get_architecture`/`get_blast_radius` (MCP
  tools) and the dashboard's `/workspace/system-map`,
  `/workspace/conformance`, `/workspace/contracts`,
  `/workspace/co-changes` views all need real cross-repo dependency
  resolution (a symbol in one repo resolving as an import/call target
  in another), which doesn't exist anywhere in this port yet — left for
  a follow-up. There's also no way to switch which repo the rest of the
  dashboard/MCP server operates on.
- `repowise-workspace` depends only on `repowise-core`, deliberately
  kept as its own crate (not folded into `repowise-core`) so a future
  cross-repo slice can grow it a `repowise-graph` dependency without
  `repowise-core` ever depending upward — a load-bearing invariant the
  rest of this port relies on.
- Uses `toml = "0.8"`, already in the dependency tree via
  `repowise-health`'s `--weights` flag — no new crate added to
  `Cargo.lock`.
- **Breaking change (internal API only):** `repowise_mcp::run` and
  `repowise_server::{app, serve}` gained a new `workspace:
  Option<PathBuf>` parameter. Every call site was updated in this same
  PR; no CLI-visible behavior change for invocations without
  `--workspace`.
- Verified end-to-end manually: a real compiled `repowise` binary
  against two scratch git repos (one indexed, one not) and a real
  workspace TOML file — the CLI subcommand, the dashboard endpoint
  (with and without `--workspace`), and the rendered Workspace section
  (screenshot via headless Chromium) all report the correct
  indexed/unindexed status per repo.
- **This does not close #64** — cross-repo tools and views remain.

---

## PR #163 — PageRank-bias /api/search ranking
**2026-07-24** · [#163](https://github.com/baileyrd/rusty_repo_wise/pull/163) · closes [#63](https://github.com/baileyrd/rusty_repo_wise/issues/63)

- **Added:** issue #63's second and final slice — `/api/search` (the
  dashboard's instant search box) now ranks substring matches by
  `repowise-graph`'s already-computed in-degree data instead of plain
  alphabetical order: files with more dependents (`dependents_of`) and
  symbols with more callers (`call_in_degree`) rank first among
  equally-matching results.
- **Deliberately not embeddings:** an API call per keystroke would make
  instant search not instant, so this is the "cheaper intermediate
  step" #63's own open questions suggested for this endpoint
  specifically — no new analysis, no network call, just re-ranking
  what the graph already knows.
- Verified end-to-end manually: a real compiled server against a
  scratch Cargo-shaped repo (`src/main.rs` with `mod b; mod c;`), where
  `src/b.rs`/`src/c.rs` (each with one dependent) correctly rank ahead
  of `src/main.rs` (zero dependents) despite `main.rs` sorting first
  alphabetically.
- **This closes issue #63.** Together with PR #161 (embeddings-based
  chat retrieval), both of #63's two open-questions paths are now
  addressed: real semantic retrieval where a chat request already
  tolerates one network round trip, and a cheaper graph-based bias
  where it doesn't.

---

## PR #161 — Add embeddings-based chat retrieval
**2026-07-24** · [#161](https://github.com/baileyrd/rusty_repo_wise/pull/161) · part of [#63](https://github.com/baileyrd/rusty_repo_wise/issues/63)

- **Added:** issue #63's first slice — real semantic retrieval for
  `/api/chat`, replacing the keyword-substring grounding it used
  before. `repowise-llm::embed` calls the configured endpoint's
  OpenAI-compatible `POST /v1/embeddings` to embed the question and
  every indexed file's symbol list in one batched request; results are
  ranked by cosine similarity and the top 10 files go into the system
  prompt.
- **Graceful fallback:** if the embeddings call itself fails (e.g. an
  endpoint that doesn't implement `/v1/embeddings` at all), chat falls
  back to the original keyword search rather than failing the request.
- **No vector index or persistence:** every chat call re-embeds the
  whole corpus in one batched request — an honest cost/latency
  tradeoff for a first slice. A larger repo would want to cache these,
  tied to the reindex job, as a follow-up.
- **`REPOWISE_EMBEDDING_MODEL`** selects the embedding model/route
  alias (default `"embed"`), separate from `REPOWISE_LLM_MODEL`.
- `/api/search` (the dashboard's instant search box) is unaffected and
  stays substring-only. #63's own open questions suggest
  PageRank-biasing that with `repowise-graph`'s in-degree data as a
  cheaper follow-up if pursued.
- **Found and fixed a real test flake** while writing this (not a
  product bug): `ureq`'s connection pooling could hand a fixture-test
  server's already-closed socket back to a second sequential request
  (fixed by setting `Connection: close` on outgoing requests), and the
  test fixture's single `read()` call didn't reliably capture a full
  HTTP request once a request body grew large enough to span multiple
  TCP segments (fixed by looping reads until the declared
  `Content-Length` is satisfied).
- Verified end-to-end manually: a real compiled server against a
  scratch git repo with two files, pointed at a small fixture
  implementing both `/v1/embeddings` (deterministic fake vectors) and
  `/v1/chat/completions`, confirming the correct file ranked first by
  similarity in the actual request sent to the LLM. Repeated with the
  fixture returning 404 on `/v1/embeddings` to confirm the keyword-
  search fallback.
- **Scope:** this doesn't close #63 — `/api/search` remains
  substring-only, and there's no vector index/persistence yet.

---

## PR #159 — Add cost tracking (token usage) to the dashboard
**2026-07-24** · [#159](https://github.com/baileyrd/rusty_repo_wise/pull/159) · closes [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** the fifth and last of #65's bundled features — cost
  tracking. `GET /api/usage` returns running chat-call and
  prompt/completion/total token counts, tallied across every
  `/api/chat` call whose response reported OpenAI-compatible `usage`.
- **Token counts, not a dollar cost:** `repowise-llm` has no per-model
  pricing table, since an OpenAI-compatible endpoint (`rusty_provider`
  or otherwise) can route to whichever provider it's configured for.
- **In-memory only:** tallied for this server process, reset on
  restart — not a persisted history across sessions, unlike real
  repowise's daily-spend/cost-heatmap view. A genuinely persisted
  history is a follow-up if ever pursued further.
- `repowise-llm` gained `complete_messages_with_usage` (capturing the
  response's `usage` object) alongside the existing
  `complete_messages`/`complete`, which are unchanged for their other
  callers (wiki-summary generation).
- `UsageSection`, the frontend component, polls `/api/usage` every 3s
  rather than fetching once, so it keeps reflecting `ChatSection`'s
  activity elsewhere on the page without the two components sharing
  state directly.
- Verified end-to-end manually: a real compiled server against a
  scratch git repo, pointed at a small fixture HTTP server returning a
  fixed OpenAI-compatible response with `usage`, `curl`-ing `/api/chat`
  then `/api/usage` to confirm the tally, then headless Chromium
  (Playwright) sending a chat message through the real UI and
  confirming the Usage section's polling loop picks up the updated
  counts (screenshot).
- **This closes issue #65**: all five of its bundled,
  live-server-dependent features (Present Mode, chat, live job banner,
  read-only Settings, cost tracking) are now delivered, across PRs
  #151, #153, #155, #157, and #159.

---

## PR #157 — Add read-only Settings view to the dashboard
**2026-07-24** · [#157](https://github.com/baileyrd/rusty_repo_wise/pull/157) · part of [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** the third of #65's remaining bundled features — a
  read-only Settings view. `GET /api/settings` returns the repo root,
  indexed file counts, whether git history and wiki pages are
  available, and whether an LLM is configured (and which model).
- **Deliberately read-only:** this port has no persisted repo-level
  exclusion/generation config or global server/webhook/MCP config to
  write to yet, so a status snapshot is this slice's honest scope
  rather than a settings editor.
- `git_available` reuses `repowise_git::GitAnalytics::collect`;
  `wiki_pages_available` reuses the existing `wiki_indexed_files`
  helper — no new detection logic, just surfacing what other endpoints
  already compute.
- `SettingsSection`, the frontend component, renders at the bottom of
  the dashboard as a plain list of status lines.
- Verified end-to-end manually: a real compiled server against a
  scratch git repo with a generated wiki page, both without and with
  `REPOWISE_LLM_BASE_URL` set, `curl`-ing `/api/settings` through both
  cases, then headless Chromium (Playwright) confirming the rendered
  section (screenshot).
- **Scope:** cost tracking (daily LLM spend, cost heatmap) remains
  undone — it needs its own design pass (persistence for cost
  history). This PR does not close #65.

---

## PR #155 — Add live job banner (reindex) to the dashboard
**2026-07-24** · [#155](https://github.com/baileyrd/rusty_repo_wise/pull/155) · part of [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** the second of #65's remaining bundled features — a live
  job banner. A "Reindex" button (top of the dashboard, next to
  Present) triggers `POST /api/reindex`, a new endpoint that kicks off
  a background reindex unless one's already running; `GET
  /api/reindex` reports the job's current status
  (idle/running/completed/failed) for the dashboard to poll.
- **Shared indexing implementation:** `build_index` (repo walk + parse)
  moved out of `repowise-cli`'s private `indexing` module into
  `repowise-parser` as a public function, so `repowise-cli`'s
  `init`/`update` and the server's new reindex job share exactly one
  implementation instead of risking drift between two copies.
- **Concurrency-safe:** an `Arc<Mutex<ReindexStatusDto>>` with an
  atomic `try_start()` guard ensures at most one reindex runs at a
  time; a bad root path surfaces as a `Failed` status, never a 500.
- `JobBanner`, the frontend component, polls `/api/reindex` via
  `gloo-timers` every 500ms until the job leaves `Running` — both on
  page load (to pick up a job already in flight from a previous visit)
  and after triggering a new one.
- Verified end-to-end manually: a real compiled server against a
  scratch git repo, `curl`-driven through the idle→running→completed
  transition, then headless Chromium (Playwright) clicking the
  "Reindex" button and observing the banner update from
  "Reindexing..." to "Indexed N file(s)..." (screenshot). Caught and
  fixed a real bug in the process: `duration_ms: u128` silently failed
  to deserialize in the WASM frontend (gloo-net's JS-number-based JSON
  path has no `u128` support), fixed by narrowing to `u64` on both
  sides.
- **Scope:** cost tracking (daily LLM spend, cost heatmap) and Settings
  (repo-level exclusions/generation options, global server/LLM/
  webhook/MCP config) remain undone — each needs its own design pass
  (persistence for cost history, a write-capable settings API). This
  PR does not close #65.

---

## PR #153 — Add Present Mode to the live dashboard
**2026-07-24** · [#153](https://github.com/baileyrd/rusty_repo_wise/pull/153) · part of [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** the first of #65's four remaining bundled features — a
  full-screen, keyboard-driven step-through of the dashboard's core
  narrative sections (Overview, Code health, Hotspots, Architectural
  decisions, Dependency graph). `ArrowRight`/`Space` advances,
  `ArrowLeft` goes back, `Escape` exits; on-screen Prev/Next buttons
  for mouse users.
- Frontend-only: no new server endpoint. Each slide reuses the
  existing section component and the same `/api/*` data it already
  fetches.
- **Shareable/bookmarkable via URL:** the current slide is reflected as
  `#present/<n>` in the URL hash via `history.replaceState` (not a
  plain hash assignment, so stepping through slides doesn't spam the
  browser's back-button history). Loading a URL with that hash present
  opens directly into that slide.
- Verified end-to-end manually: entered Present Mode on a real running
  server, confirmed the URL hash updates on `ArrowRight`/`ArrowLeft`
  with the correct slide rendering (screenshots), confirmed `Escape`
  clears the hash and exits, and confirmed reloading directly at
  `#present/3` opens straight into that slide.
- **Scope:** cost tracking (daily LLM spend, cost heatmap), Settings
  (repo-level exclusions/generation options, global server/LLM/
  webhook/MCP config), and the live job banner (background indexing
  progress) remain undone — each needs its own design pass
  (persistence, a write-capable settings API, a background-job concept
  the server doesn't have at all yet). This PR does not close #65.

---

## PR #151 — Add dashboard chat view over repowise-llm (Phase 5)
**2026-07-24** · [#151](https://github.com/baileyrd/rusty_repo_wise/pull/151) · closes [#59](https://github.com/baileyrd/rusty_repo_wise/issues/59) · part of [#65](https://github.com/baileyrd/rusty_repo_wise/issues/65)

- **Added:** Phase 5 of the dashboard-server pivot — a chat view over
  `repowise-llm`. Every view the static `repowise dashboard` page had
  now has a live equivalent, plus drill-down, search, a dependency
  graph, and chat the static page never had.
- **`repowise-llm`:** adds multi-turn support (`Turn`,
  `complete_messages`); the existing single-turn `complete()` becomes a
  thin wrapper over it, no breaking change to wiki-summary generation.
- **New `repowise-server` endpoint:** `POST /api/chat` takes
  `{"history": [...]}` (the whole conversation, client-owned) and
  returns `{"available": bool, "reply": string | null}` —
  `available: false` when `REPOWISE_LLM_BASE_URL` isn't set, the same
  opt-in convention `repowise generate` (#61) uses. The latest user
  message is grounded with a lightweight keyword search over indexed
  files/symbols before reaching the LLM — not real embeddings-based
  retrieval, which is #63's job. `LlmConfig` is now resolved once at
  server startup into `AppState` rather than re-read per request, so
  tests can inject a fixture config without racing on process env vars.
- **`repowise-web`:** a new chat section keeps history client-side and
  resends it every turn; shows a plain explanatory message instead of a
  broken-looking chat box when the server reports the feature isn't
  configured.
- Verified end-to-end manually: ran the real compiled server with
  `REPOWISE_LLM_BASE_URL` pointed at a throwaway fake OpenAI-compatible
  endpoint, confirmed a grounded reply over `curl` and then in a real
  browser across two chat turns (history accumulates correctly), then
  restarted with the env var unset and confirmed the "not configured"
  message renders instead of a chat box.
- **Closes #59** (live/instant search — delivered in Phase 2, and this
  PR's chat view was the last item blocking full closure of the
  dashboard-server pivot itself).
- **Does not close #65**: that issue bundles five live-server-dependent
  features (Present Mode, chat, cost tracking, settings, live job
  banner). Only chat is done; #65 was reopened after this PR's merge
  incorrectly auto-closed it, and is now rescoped to the four remaining
  items.

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
