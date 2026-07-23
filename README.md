# rusty_repo_wise

A Rust-native reimplementation inspired by [repowise](https://github.com/repowise-dev/repowise),
a codebase-intelligence platform that builds dependency graphs, git
analytics, auto-generated docs, architectural-decision tracking, and
deterministic code-health scoring for AI agents and developers.

This is a fresh, from-scratch Rust project — it does not share code or a
license with the original (AGPL-3.0) repowise.

## Scope so far

repowise is a large product with five "intelligence layers," an MCP
server, and a web dashboard. This port now builds all of that — every
piece covering a subset of the original's scope within it (see below for
specifics per layer), not full feature parity:

- Walk a codebase (respecting `.gitignore`), detect Rust, Python,
  TypeScript, JavaScript, Java, Kotlin, Go, C, C++, C#, Scala, Ruby,
  Swift, PHP, Dart, and shell (`.sh`/`.bash`/`.zsh`) files.
- Parse each file with tree-sitter, extracting function/method/class/struct
  definitions, imports, call expressions, and per-function metrics
  (cyclomatic complexity, parameter count, a duplicate-code body hash).
- Resolve imports and calls into a dependency graph (files and symbols as
  nodes; `Contains`/`Imports`/`Calls` edges), using directory-layout
  conventions (Rust's `mod`/crate-root rules, Python's package layout,
  TypeScript/JavaScript's relative `./`/`../` specifiers, Java/Kotlin/
  Scala's shared Maven/Gradle/sbt `src/main/java`/`src/main/kotlin`/
  `src/main/scala`-anchored package paths, Go's `go.mod`-anchored module
  paths, C/C++'s quote-form `#include "local.h"` resolved directly
  against the filesystem, C#'s best-effort folder-mirrors-namespace
  heuristic, Ruby's `require_relative` resolved directly against the
  filesystem) — **not** full compiler-grade name resolution. Ambiguous or
  external references (npm packages/JVM classpath dependencies/Go modules
  outside the local `go.mod`/C and C++ angle-form `#include <system>`
  headers/C# namespaces that don't follow the folder convention/Ruby's
  gem-based plain `require`/PHP namespaces that don't follow the folder
  convention, since there's no
  `node_modules`/classpath/Go-proxy/include-path-search/.NET-project/
  `$LOAD_PATH`/Composer-autoload resolution) are left unresolved rather
  than guessed. `.h` is deliberately not mapped to either C or C++
  (ambiguous between the two, and this port has no syntax-sniffing to
  disambiguate) — so a conventional `#include "foo.h"` split resolves
  against the filesystem at parse time but never becomes a graph edge,
  since the header itself is never indexed; only unambiguous extensions
  (`.c`, and C++'s `.cpp`/`.cc`/`.cxx`/`.hpp`/`.hh`/`.hxx`) are
  recognized. Swift's `import` is module-level rather than file-level (no
  relative-import syntax, and a module name has no file mapping without a
  full build graph) — its imports are recorded for visibility but always
  left unresolved, by design rather than as a gap. PHP has two import
  forms: `require`/`require_once`/`include`/`include_once` with a plain
  string path (resolved directly against the filesystem, same as
  C/C++/Ruby) and `use Namespace\Class;` (resolved via the same
  folder-mirrors-namespace heuristic as C#'s, not aware of Composer's
  real `composer.json` autoload mapping). Dart's relative
  `import 'local.dart'` is resolved directly against the filesystem the
  same way; `import 'package:x/y.dart'` (a pub package) has no package
  registry here to resolve against and is left unresolved, same
  tradeoff as bare npm specifiers. Shell's `source`/`.` is likewise
  resolved directly against the filesystem, including the common
  `SCRIPT_DIR="$(dirname "$0")"` / `source "$SCRIPT_DIR/helper.sh"`
  idiom for a script sourcing something relative to its own directory;
  any other variable/command-substitution in the path has no static
  value to resolve, so it's recorded but left unresolved.
- Score every file's health deterministically (0–10, no LLM/ML) from twelve
  rule-based markers: long functions, high cyclomatic complexity, oversized
  parameter lists, god classes, duplicate code, near-duplicate code
  (`dry_violation` — Rabin-Karp rolling-hash overlap over tokenized
  text), possibly-dead code (zero resolved callers), low cohesion
  (LCOM4 — Rust/Python/TS+JS only, see "Health scoring" below), nested
  complexity (`nested_complexity` — maximum control-flow nesting depth,
  complementing cyclomatic complexity's flat branch count), a
  "bumpy road" (`bumpy_road` — count of distinct nested-block regions,
  complementing nesting depth's single deepest-point view), complex
  conditionals (`complex_conditional` — a single `if`/`while`/etc. condition
  chaining 3+ boolean operators, Rust/Python/TS+JS only), and primitive
  obsession (`primitive_obsession` — a parameter list leaning on bare
  primitives instead of domain types, Rust/TypeScript only since it needs
  declared parameter types) — except for shell scripts, which are
  deliberately exempt from the dead-code
  marker: a shell function is routinely invoked only from the command
  line, another script, or a cron job, none of which this port's call
  graph can see, making the signal too unreliable to report for that
  language.
- Derive git-history analytics — churn, hotspot score (churn × complexity),
  bug-fix commit frequency, co-change coupling, and per-author line
  ownership — by shelling out to `git log`/`git blame`, joined against the
  index for complexity data.
- Generate a deterministic, template-based markdown "wiki" page per file
  under `.repowise/wiki/` — symbol list, resolved dependencies/dependents,
  and health findings — with per-file freshness tracking (no LLM prose).
- Mine architectural decisions from `docs/adr/*.md` files and decision-like
  commit messages, link each to the files/symbols it mentions, and track
  supersession via an ADR's `Status: Superseded by ADR-XXXX` line.
- Expose `get_overview`/`search_codebase`/`get_context`/`get_risk`/
  `get_change_risk`/`get_symbol`/`get_why`/`get_dead_code` as MCP tools
  over stdio (the official `rmcp` SDK), so an agent can pull complete
  context (including git-history risk data, a deterministic per-commit
  risk score, a symbol's raw source, the architectural decisions behind a
  file, and confidence-tiered dead-code candidates) for a file, a change,
  a single symbol, "why was this built this way", or "what looks unused"
  in one round-trip instead of piecing it together itself.
- Generate a static-site dashboard (one self-contained HTML page, no
  server, no JS build step) covering overview stats, code health,
  hotspots, and mined decisions — regenerate by re-running the command.
- Persist the index to `.repowise/index.json` and query it from the CLI.

Only Rust, Python, TypeScript, JavaScript, Java, Kotlin, Go, C, C++, C#,
Scala, Ruby, Swift, PHP, Dart, and shell scripts are parsed; repowise's
other languages aren't implemented — see issue #11 for the
tracking/discussion issue on extending language support. The health scorer covers 12 of repowise's ~25 markers — see
"Health scoring" below for which ones and why the rest (the
ML-calibrated organizational-signal markers) are deferred. LLM-written prose on
top of the wiki (`repowise generate` in the original) is also deferred —
this port's `docs` layer is deliberately deterministic-only, as is ADR
mining (only 6 of the original's 8 decision sources are implemented —
see "Architectural decision mining" below). The MCP server covers 8 of
the original's ~10 tools — see "MCP server" below for which and why. The
dashboard is one static page with no per-file drill-down or live search
— see "Dashboard" below for what a richer version would need.

## Crates

- `repowise-core` — shared data model (`Symbol`, `FileRecord`, `RepoIndex`,
  etc.), `.gitignore`-aware file discovery, and JSON index persistence.
- `repowise-parser` — tree-sitter-based extraction for Rust, Python,
  TypeScript, JavaScript, Java, Kotlin, Go, C, C++, C#, Scala, Ruby,
  Swift, PHP, Dart, and shell scripts, including per-function
  complexity/nesting-depth/bumpy-road/param-count/body-hash metrics, plus
  per-method `self`/`this` field-access tracking for Rust/Python/TS+JS
  (feeds LCOM4), per-condition boolean-operator-chain detection for
  Rust/Python/TS+JS (feeds `complex_conditional`), and declared-parameter-type
  extraction for Rust/TypeScript (feeds `primitive_obsession`).
- `repowise-graph` — builds the dependency graph from a `RepoIndex` and
  answers overview/search/deps/call-in-degree queries.
- `repowise-health` — deterministic code-health scoring built on top of
  the parsed metrics and the call graph, including LCOM4 low-cohesion
  detection over per-class field-access data and Rabin-Karp near-duplicate
  detection over source text re-read from disk.
- `repowise-git` — git-history analytics (churn, hotspots, bug-fix
  frequency, co-change coupling, ownership), computed fresh from `git
  log`/`git blame` each time it's queried rather than cached in the index.
- `repowise-docs` — deterministic per-file markdown documentation pages
  rendered from the index/graph/health data, with content-hash-based
  freshness tracking.
- `repowise-adr` — architectural-decision mining from ADR files,
  decision-like commit messages, decision-like merged PR bodies (via the
  GitHub API, opt-in behind a token env var), decision-like code
  comments, inline decision markers, and keep-a-changelog-style
  CHANGELOG sections, linked to the files/symbols they mention.
- `repowise-mcp` — an MCP server (via the official `rmcp` SDK) exposing
  the index/graph/health/git-analytics/mined-decisions data, plus a
  deterministic per-commit change-risk score and confidence-tiered
  dead-code candidates, as agent-facing tools over stdio.
- `repowise-dashboard` — a static-site dashboard rendered from the
  overview/health/hotspot/decision data the other layers compute.
- `repowise-cli` — the `repowise` binary tying it together.

## Usage

```sh
cargo build --release

repowise init [PATH]              # build a fresh index (default PATH: .)
repowise update [PATH]             # re-index (currently a full re-index)
repowise overview [PATH]           # summary stats: languages, symbols, edges
repowise search "<query>"  [PATH]  # substring search over symbol names
repowise deps <FILE> [PATH]        # a file's resolved dependencies/dependents
repowise health [PATH]             # code-health KPIs and lowest-scoring files
repowise hotspots [PATH]           # files ranked by churn × complexity
repowise ownership <FILE> [PATH]   # per-author line ownership (git blame)
repowise coupled <FILE> [PATH]     # files that most often change alongside it
repowise docs [PATH]               # generate per-file wiki pages under .repowise/wiki
repowise decisions [PATH]          # mined ADRs + decision-like commits, with linked files
                                    #   --for-file <FILE> to filter to one file
repowise serve [PATH]               # run an MCP server over stdio (get_overview/search_codebase/get_context/get_risk/get_change_risk/get_symbol/get_why/get_dead_code)
repowise dashboard [PATH]           # generate a static HTML dashboard under .repowise/dashboard
```

## Health scoring

`repowise health` requires a prior `init`/`update`. Each file starts at a
score of 10.0 and loses points for every marker triggered in it, clamped
to `[0, 10]`:

| Marker | Threshold | Penalty |
|---|---|---|
| Long function | > 50 lines | −0.5 |
| High cyclomatic complexity | > 10 | −1.0 |
| Too many parameters | > 6 | −0.3 |
| God class | > 15 methods | −1.5 |
| Duplicate code | body hash matches another symbol's | −0.5 |
| Near-duplicate code (`dry_violation`) | >= 50% tokenized-window overlap with another symbol | −0.3 |
| Possibly dead code | 0 resolved callers | −0.2 |
| Low cohesion (LCOM4) | >= 2 disjoint field-access groups | −1.0 |
| Nested complexity (`nested_complexity`) | control flow nested > 4 levels deep | −1.0 |
| Bumpy road (`bumpy_road`) | >= 3 separate nested-block regions | −0.5 |
| Complex conditional (`complex_conditional`) | single condition chains >= 3 boolean operators | −0.3 |
| Primitive obsession (`primitive_obsession`) | >= 3 bare-primitive-typed parameters | −0.3 |

"Possibly dead code" is never applied to shell scripts (`Language::Shell`)
— a shell function is routinely invoked only from the command line,
another script, or a cron job, none of which this port's call graph can
see, so the signal is too unreliable to report for that language. All
other markers still apply to shell the same as everywhere else.

All of these come from data already computed by `repowise-parser`
(per-symbol line span, complexity, param count, body hash) and
`repowise-graph` (call-graph in-degree) — no new heuristics are hidden
inside the scorer itself. "Possibly dead code" and "duplicate code" are
intentionally low-weighted since they inherit the graph layer's
best-effort call/import resolution: a symbol can look uncalled just
because a call site couldn't be resolved (trait dispatch, dynamic
dispatch, an external caller), not because it's truly unused.

**Low cohesion (LCOM4)** builds a per-class graph — methods are nodes,
an edge connects two methods that both access at least one common field
— and flags a class whose field-touching methods split into 2+ disjoint
groups (no shared field access between groups at all). Field-access
extraction (`self`/`this` field reads/writes) is currently implemented
for **Rust, Python, and TypeScript/JavaScript only** — the three
languages issue #51's own acceptance criteria named explicitly, out of
the 16 languages this port otherwise parses; the other 13 have an empty
field-access list per file and are silently skipped for this one marker
(not enough data, not "cohesive"), not flagged. A method that never
touches a field of its own (a pure delegator, a static-style helper) is
excluded from the per-class graph entirely rather than counted as its
own singleton component — otherwise nearly any real-world class would
trip this marker. Extending field-access tracking to the remaining
languages is a natural follow-up, not done here.

**Nested complexity (`nested_complexity`)** measures maximum control-flow
nesting depth (if/for/while/etc. nested inside each other) per function,
complementing cyclomatic complexity: a function with 10 sequential ifs
and one with the same 10 ifs nested inside each other score identically
on cyclomatic complexity but read very differently, and only nesting
depth tells them apart. Computed by `repowise-parser::metrics::max_nesting_depth`
alongside the existing `cyclomatic_complexity` — same recursive
decision-node classification per language, just tracking depth reached
rather than a flat count — for **all 16 parsed languages** (unlike
LCOM4, this needed no new per-language extraction logic, since every
language's `is_decision` classification already existed for cyclomatic
complexity).

**Bumpy road (`bumpy_road`)** complements `nested_complexity`: rather
than the single deepest point reached, it counts how many *separate*
nested-block regions occur within one function — three separate
two-level-deep blocks read worse than one two-level-deep block, even
at the same max nesting depth, and `max_nesting_depth` alone can't
tell them apart. Computed by `repowise-parser::metrics::bumpy_road_bumps`,
also alongside `cyclomatic_complexity`/`max_nesting_depth` for all 16
languages. Counting rule: only *leaf* decision nodes count (a decision
node with no further decision node nested inside it, before hitting a
nested-function boundary) that reach a nesting depth of at least 2 —
a linear chain (`if` containing `if` containing `if`) has exactly one
leaf and counts as a single bump, not three, since it's one deep block
rather than several scattered ones; three separate sibling `if`s each
with one level of nesting inside have three leaves and count as three
bumps. Flagged at 3+ bumps.

**Complex conditional (`complex_conditional`)** flags a single `if`/
`while`/etc. condition that chains 3+ boolean operators (`&&`/`||` in
Rust/JS/TS, `and`/`or` in Python) — unlike `nested_complexity` and
`bumpy_road`, which are Symbol-level aggregate scalars, this marker needs
to point at the *specific condition*, not just the enclosing function, so
each flagged condition is its own `Finding` with its own line number
(`Symbol::complex_conditionals: Vec<ComplexConditionalRef>`, each entry
carrying the condition's own `line` and its `operator_count`). Extraction
is implemented for **Rust, Python, and TypeScript/JavaScript only** — the
same three languages LCOM4 and near-duplicate detection require new
per-language grammar logic for — via a `condition_of` closure per language
that pulls the `condition` sub-expression out of an `if`/`while`/etc. node,
and a separate `is_boolean_operator` closure (deliberately distinct from
each language's existing `is_decision` classifier) that counts chained
boolean operators within just that condition's own subtree, not the whole
function body. The other 13 languages have no per-language
`condition_of`/`is_boolean_operator` logic yet and so never produce any
entries for this marker.

**Primitive obsession (`primitive_obsession`)** flags a function/method
whose declared parameters lean on bare primitives (`i32`/`bool`/`String`
and language equivalents) rather than small domain-specific types — the
classic "primitive obsession" smell, where a handful of loosely-related
primitive values would read better bundled into their own type. This
needs actual declared parameter *types*, which only exist for
statically-typed languages in this port's model, so it's implemented for
**Rust and TypeScript only** for this first pass (`Symbol` gains
`primitive_param_count: usize`, counting declared parameters whose type
resolves to a bare primitive). For Rust, a leading `&`/`&mut`/lifetime
reference prefix is stripped before classifying (`&str` and `String`
both count), and `String`/`str` are included alongside the scalar keyword
types even though `String` isn't a `Copy` primitive in Rust's own type
system — the smell targets overused strings/ints/bools, not Rust's
`Copy` boundary. For TypeScript, only `string`/`number`/`boolean` count
(not `any`/`unknown`/`void`/etc.). The other 14 parsed languages
(including Python/JavaScript, which lack static type annotations in the
common case and would need type inference this port doesn't have) get an
empty parameter-type extraction and so never trigger this marker;
extending it to the remaining statically-typed languages (Java, Kotlin,
Go, C, C++, C#, Scala, Swift, Dart) is a natural follow-up, not done here.

**Near-duplicate code (`dry_violation`)** catches *partial* duplicates
the exact-hash `Duplicate code` marker misses entirely — a function
that's mostly identical to another with a few renamed variables or a
tweaked constant, where even one differing character breaks a hash
match. Rather than growing `Symbol`/`FileRecord` with raw body text just
for this, it re-reads each candidate symbol's source fresh from disk
(the same tradeoff `get_symbol` and the ADR code-comment/inline-marker
sources already make elsewhere in this port) and tokenizes it
(identifier/number runs plus single-character punctuation), then splits
each symbol's token sequence into overlapping 3-token windows, hashed
with an incremental Rabin-Karp rolling hash. Windowing over *tokens*
rather than raw characters matters because a renamed identifier changes
length — `total` -> `sum` shifts every subsequent character position,
which would misalign every raw-character window from that point on even
though the code is otherwise identical; a token-level window only loses
the windows actually touching the renamed token. Two symbols become a
candidate pair the moment they share one window hash (avoiding
brute-force all-pairs comparison), then are flagged once their shared
window count reaches 50% of the smaller symbol's total — pairs already
caught by the exact-hash `Duplicate code` marker (identical `body_hash`)
are explicitly excluded so a pair is never reported under both finding
kinds at once.

Deferred markers from the original repowise (not implemented): the
ML-calibrated organizational-signal markers (`churn_risk`,
`co_change_scatter`, etc. — see issue #62, a design-level "needs-human"
question, not a mechanical gap). Hotspots and bug-fix history are now
implemented (see "Git analytics" below) but aren't yet folded into the
health score itself — that's a natural follow-up, not done here.

## Git analytics

`repowise hotspots`/`ownership`/`coupled` shell out to `git log`/`git
blame` under the hood — no persistence, no caching, just re-run each
time against the repo's real history:

- **Churn**: number of commits touching a file, from a single `git log
  --name-only` walk of the whole history.
- **Hotspot score**: `churn × total cyclomatic complexity` of the file's
  symbols (complexity already computed by `repowise-parser`). `hotspots()`
  ranks files by a **recency-weighted** variant of this score: each commit
  contributes `exp(-age_days / 90)` instead of a flat `1`, so a file with
  the same raw churn as another but touched more recently ranks higher.
  The raw (non-decayed) score is still reported alongside it for
  transparency.
- **Bug-fix commits**: commits whose message contains a fix-like keyword
  (`fix`, `bug`, `hotfix`, `patch`) touching the file. A heuristic, not
  ground truth — a fix described without one of these words won't be
  counted, and an unrelated commit that happens to mention one will be.
- **Co-change coupling**: files that appear together in the same commit,
  counted per pair. Commits touching more than 50 files are skipped when
  building this (a rename sweep or vendor bump would otherwise flood
  every touched file's coupling list with noise).
- **Ownership**: per-author share of a file's lines from `git blame
  --line-porcelain`.

Not implemented from the original's git-analytics scope: a bug-fix
heuristic based on linked-issue references rather than just message
keywords.

## Documentation generation

`repowise docs` renders one markdown page per indexed file under
`.repowise/wiki/<relative-path>.md` (e.g. `crates/foo/src/lib.rs` →
`.repowise/wiki/crates/foo/src/lib.rs.md`), each containing:

- Its symbol list (function/method/class/struct, with parent and line number)
- Resolved dependencies and dependents (from `repowise-graph`)
- Its health findings (from `repowise-health`), or "No findings."

No LLM is involved — every page is rendered from data the other layers
already computed. Freshness is tracked via a hash of the file's own raw
source, embedded as the page's first line and compared against the
previous run's page (if any) to report each page as new/changed/
unchanged. This is a **per-file, own-source-only** signal: a page can be
reported "unchanged" while its actual rendered content differs, if what
changed was cross-file data (a new caller elsewhere, a health finding
driven by another file) rather than this file's own source — pages are
always rewritten with current data regardless of the reported status, so
content is never stale, only the *status label* can undersell how much
changed. Not implemented from the original: LLM-written prose on top of
these pages (`repowise generate`), and the dashboard's doc browser.

## Architectural decision mining

`repowise decisions` mines six of the original's eight decision sources:

- **`docs/adr/*.md` files**, parsed against this repo's own ADR template
  (`# ADR-XXXX: Title`, then `Status:`/`Date:` lines). An unfilled
  template (title still literally `<Title>`) is skipped rather than mined
  as a real decision.
- **Decision-like commit messages** — a message containing one of a
  19-verb keyword set (`decide`, `decision`, `chose`, `chosen`,
  `switch to`, `adopt`, `instead of`, `migrate`, `replace`, `deprecate`,
  `drop`, `rewrite`, `split`, `revert`, `opt for`, `in favor of`,
  `settle on`, `consolidate`, `standardize on`). A heuristic, not ground
  truth, same framing as the bug-fix-commit detection in git analytics.
- **Decision-like merged PR bodies** — the same keyword heuristic as
  commit messages, applied to a merged PR's title/body via the GitHub
  API. This is the one decision source (and the one place in
  `repowise-adr`) that makes a network call, and only when a
  `REPOWISE_GITHUB_TOKEN` environment variable is set: a local
  codebase-analysis CLI making unsolicited outbound HTTP requests would
  be surprising, so this is opt-in rather than falling back to GitHub's
  unauthenticated (and much more rate-limited) API. No token, no git
  remote, or a remote that isn't GitHub-hosted all degrade to "this
  source found nothing" rather than erroring — same tradeoff the other
  two sources already make for a missing `docs/adr/` or unreadable git
  history. Unlike the other two sources, a PR decision links to the
  files that PR actually touched (already reported by the GitHub API)
  rather than falling back to text-matching.
- **Decision-like code comments** — the same keyword heuristic again,
  applied to the comment/docstring block sitting *directly above* an
  indexed symbol's declaration (`///`/`/** */` above a Rust/Java/
  C-family declaration, `#`-prefixed lines above a Python/Ruby function).
  Pure filesystem/parsing, no new dependency, unlike the PR-body source.
  Deliberately scoped to that one convention — Python/JavaScript's
  alternative of a docstring as the function body's first statement
  isn't handled, a documented gap rather than a silent one. Linked to
  the file the comment sits in, the same "authoritative, not
  text-matched" treatment PR decisions get.
- **Inline decision markers** — a small, explicit tag vocabulary
  (`WHY:`, `DECISION:`, `TRADEOFF:`, `ADR:`, `RATIONALE:`, `REJECTED:`)
  recognized as a prefix inside any comment syntax (`#`, `//`, `/* */`),
  wherever it appears in a file — not tied to sitting above a particular
  symbol the way the code-comment source is. Much lower false-positive
  risk than the freeform code-comment source: this is an explicit opt-in
  convention, not a keyword guess, so every match is deliberate. A plain
  text scan (not language-specific parsing), same "pure filesystem work,
  no new dependency" framing as the code-comment source. Linked to the
  file the marker sits in.
- **Keep-a-changelog-style CHANGELOG sections** — `CHANGELOG.md`/
  `HISTORY.md`/`NEWS.md`/`CHANGES.md` at the repo root (whichever is
  found first, case-insensitive), scanning for `### Changed`/
  `### Removed`/`### Deprecated`/`### Security` section headings (a
  heading-text match, not a full keep-a-changelog spec parser).
  `### Added`/`### Fixed` are deliberately excluded — purely additive or
  bug-fix entries aren't architectural decisions the way a
  change/removal/deprecation/security call generally is. Pure
  filesystem/parsing, no new dependency. Unlike the PR-body/code-comment/
  inline-marker sources, a changelog entry is linked the same way
  ADR-file/commit-message decisions are (text-matched against the
  index) rather than an authoritative self-link to the changelog file:
  the changelog file itself isn't what the decision is *about*, unlike a
  PR's diff or the file a comment sits in.

Each ADR-file/commit-message/changelog decision is linked to the indexed
files it mentions: either the file's own relative path appearing
verbatim in the decision's body text, or one of its non-module symbol
names (4+ characters, to cut down on false positives from short
identifiers) appearing as a whole word. Matching text, not meaning — a
decision that only refers to a file descriptively ("the queue module")
won't be linked. Supersession is read directly from an ADR's
`Status: Superseded by ADR-XXXX` line — no new front-matter convention
was needed since the
existing template already has one.

Not implemented from the original's eight sources: Slack and issue
trackers — this repo doesn't have integrations for either anyway.
Recency/confidence scoring on mined decisions is also not implemented.

## MCP server

`repowise serve [PATH]` runs an MCP server over stdio (via the official
[`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) SDK), requiring
a prior `repowise init`/`update`. Eight tools are implemented:

- **`get_overview`** — the same data as `repowise overview`: file/language/
  symbol counts, edge counts, most-depended-on files.
- **`search_codebase`** — the same substring search as `repowise search`.
- **`get_context`** — a file's symbols, resolved dependencies/dependents,
  and health score/findings in one call. This is the tool that matters
  most for the original's stated goal (cutting an agent's token spend on
  context-loading): one round-trip instead of separate search/deps/health
  reads pieced together by the caller.
- **`get_risk(file?, top_n?)`** — `get_context` plus git-history risk
  data: hotspot score, churn, and bug-fix-commit count from
  `repowise-git`. Given `file`, returns that file's risk profile alone;
  given no `file`, returns the `top_n` riskiest files repo-wide, ranked
  by (recency-weighted) hotspot score. Degrades to zero/empty git data
  (rather than erroring) when the indexed root isn't a git repository —
  same tradeoff `repowise-dashboard`'s hotspots section already makes.
- **`get_change_risk(revspec?)`** — a deterministic 0-10 risk score for a
  single commit or a `base..head` range (defaulting to `HEAD`), computed
  from diff-shape metrics via `git diff`/`git show`/`git rev-list`: lines
  added/deleted, files touched, subsystems (top-level directories)
  touched, change concentration (a normalized Shannon entropy of how
  evenly the diff is spread across files), and the head commit author's
  prior-commit count as an experience proxy. These combine via a fixed,
  documented weighting (0.25 lines, 0.20 each for files/subsystems/
  author, 0.15 concentration) into the final score. **This is not the
  original's tool** — the original feeds the same kind of diff-shape
  metrics into a pre-trained L2-logistic-regression model; this port has
  no labeled defect corpus or training pipeline to reproduce that (see
  issue #42 and the category-A "ML-calibrated scoring" issue), so this is
  a deliberately simple heuristic approximation, not a calibrated
  probability. Errors (rather than degrading to zero) when the indexed
  root isn't a git repository, since there's no diff to compute at all.
- **`get_symbol(symbol_id, context_lines?)`** — a symbol's raw source
  text, sliced from its own file at the `start_line..end_line` span
  `search_codebase`/`get_context` report (both now include each symbol's
  `id`). `context_lines` (default 0) pads that span by the same number of
  lines on each side, clamped to the file's actual bounds rather than
  erroring on an out-of-range request. Re-reads the file fresh from disk
  each call, the same "don't trust the index for content, only for line
  numbers" tradeoff `repowise-docs`'s freshness tracking already makes —
  so edits since the last `init`/`update` are reflected, at the cost of
  the returned span possibly being off if line numbers have since shifted.
- **`get_why(targets?)`** — architectural decisions mined from
  `docs/adr/*.md`, decision-like commit messages, decision-like merged PR
  bodies, decision-like code comments, inline decision markers, and
  keep-a-changelog-style CHANGELOG sections (via `repowise-adr`), the
  same data as `repowise decisions --for-file`. `targets` is a list
  of file paths or symbol ids (mixing both is fine — a symbol id resolves
  to its own file); a decision is returned if its body links to at least
  one target's file. Omit `targets` (or pass an empty list) to get every
  mined decision. A thin wrapper with no new mining logic of its own —
  the same "reuse an existing library call" shape as `get_overview`/
  `search_codebase`.
- **`get_dead_code(min_confidence?, safe_only?, limit?)`** — functions/
  methods with zero resolved in-repo callers (the same base signal as the
  `possibly-dead-code` health marker), tiered `low`/`medium`/`high` by two
  cheap risk factors: an ambiguous same-named symbol elsewhere in the
  repo (a call meant for this one could have resolved to that one
  instead), and an unresolved import elsewhere whose last path segment
  matches this file's stem (something may have meant to import this file
  but this port's heuristics couldn't confirm it). Zero risk factors ->
  `high`; one -> `medium`; both -> `low`. `min_confidence` filters to
  that tier and above; `safe_only` narrows to `high` only — the closest
  this tool gets to the original's "safe to delete" designation, though
  it explicitly is **not** a runtime-safety guarantee at any tier:
  reflection, dynamic dispatch, and entry points are all invisible to
  this port's static call graph, the same caveat the `possibly-dead-code`
  marker already carries. `limit` (default 50) caps the returned list;
  the response's `total_matching` reports how many candidates matched
  before truncation. This is a documented approximation of the
  original's dead-code model (4 finding kinds, 3 confidence tiers, and a
  runtime-load risk factor this port has no way to assess) — see
  `repowise_health::find_dead_code` for the exact tiering logic.

Every call re-loads `.repowise/index.json` and rebuilds the dependency
graph fresh — no caching across calls, consistent with how `hotspots`/
`ownership`/`coupled`/`decisions` already work in this port.
(`get_change_risk` doesn't touch the index at all — it's pure `git`
plumbing, same as `repowise-git`'s other functions.)

Not implemented from the original's ~10 tools: the rest of the original's
tool surface beyond what this port's other layers currently support.

## Dashboard

`repowise dashboard [PATH]` writes one self-contained static HTML page to
`.repowise/dashboard/index.html` — open it directly in a browser, no
server to run. Kept deliberately simple: a single page combining four
sections, each degrading gracefully to an explicit "not available"
placeholder (never a silently blank section) when its data doesn't exist:

- **Overview** — same data as `repowise overview`.
- **Code health** — average score, markers by kind, lowest-scoring files
  (same data as `repowise health`).
- **Hotspots** — top files by churn × complexity (same data as `repowise
  hotspots`), or a placeholder if `PATH` isn't a git repo.
- **Architectural decisions** — mined ADRs/decision-commits (same data as
  `repowise decisions`), or a placeholder if none are found.

Regenerating means re-running the command — there's no live server, no
auto-refresh, and no per-file drill-down (e.g. clicking a file to see its
`repowise-docs` wiki page). A richer version would need at minimum: a
small local HTTP server (the `tokio` dependency already exists from the
MCP server) for live queries instead of a static snapshot, and linking
each file mentioned to its rendered wiki page. Deliberately left out of
this first pass to keep the dashboard to what "generate a static site
from data we already compute" actually requires.

## Testing

```sh
cargo test --workspace
```

Includes parser unit tests (function/class/import/call/complexity/
param-count/duplicate-hash extraction on inline source snippets), graph
integration tests that write real fixture files to a temp directory to
exercise Rust's `mod`/crate-root resolution and Python's package-relative
import resolution end to end, health-scoring tests that build
`RepoIndex` fixtures directly to exercise each marker (and the resulting
score) in isolation, git-analytics tests that build real disposable git
repos (via the `git` CLI) to exercise churn/bug-fix/co-change/ownership/
hotspot computation against actual `git log`/`git blame` output rather
than a mock of it, docs-generation tests covering page rendering content
and the New/Changed/Unchanged freshness transitions on a real temp
directory, ADR-mining tests (ADR parsing, the unfilled-template skip,
decision-commit detection, file/symbol linking, and an end-to-end test on
a real git repo covering supersession and linking together), and MCP
server tests that call each tool method directly against a real index
built by the actual indexing pipeline (not hand-built fixtures), covering
the happy path for all three tools plus the invalid-query and
unindexed-file error cases, and dashboard tests covering HTML escaping,
relative-path rendering, the graceful-degradation placeholders, and an
end-to-end render against a real indexed temp directory.
