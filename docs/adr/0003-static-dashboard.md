# ADR-0003: A static, committed-payload dashboard

Status: Proposed
Date: 2026-08-05

Tracks issue #383.

## Context

The dashboard requires a running server. `repowise serve-dashboard`
starts an axum backend and the `repowise-web` Leptos/WASM frontend calls
`/api/*` for every view, so seeing it means a checkout, a built index,
and a live process.

[Understand-Anything](https://github.com/Egonex-AI/Understand-Anything)'s
dashboard instead runs entirely off committed JSON with no API calls at
all. A link beats "clone this, build it, run this, open localhost" — and
that gap is the reason #383 exists.

### What measuring changed

#383 was written assuming a static dashboard would fetch the portable
index (6.32 MB after #381) and that **#381 was a prerequisite**. Both
assumptions are wrong, and the second is wrong in a way that would have
shaped the design badly.

The dashboard's frontend never consumes the index. It consumes 32
endpoint responses, which are *summaries*. Measured against this repo,
serving live:

| Payload | Bytes | Share |
| ------- | -----:| -----:|
| 22 aggregate views, all of them | 545,541 | 16% |
| 2,913 symbol details | 830,205 | 25% |
| 171 decision details | 2,015,748 | **59%** |
| **Total static bundle** | **3,391,494 (3.23 MB)** | |
| Portable index, for comparison | 6,623,810 (6.32 MB) | 2.0× larger |

Two consequences:

1. **A static dashboard should not ship the index at all.** It should
   ship pre-computed view responses. That is half the bytes and none of
   the client-side computation.
2. **#381 was not a prerequisite.** Interning caller ids was worth doing
   on its own merits, but it does not gate this. The dependency named in
   #383 and #384 was assumed, not measured.

### What can actually be static

The second assumption worth discarding is that git- and ADR-derived
views must be absent. **The exporter runs on a machine that has the
checkout.** It can compute hotspots, churn, ownership, contributors,
commits, and mined decisions at export time and bake them in — they are
read-only snapshots, exactly like everything else in the bundle.

Classifying all 32 handlers by what they need at *request* time (derived
from the code, with per-function boundaries — a first coarse pass using
a fixed line window misclassified at least two of four spot-checks and
was discarded):

| Class | Count | Static? |
| ----- | -----:| ------- |
| Index-only | 17 | yes, directly |
| Git-derived (`hotspots`, `commits`, `coupling`, `ownership`, `contributors`, `stats`, `commit-risk`, `health`, `settings`) | 9 | yes, pre-computed at export |
| ADR-derived (`decisions`, `decision`) | 2 | yes, pre-computed at export |
| Filesystem (`wiki`, `doc-coverage`) | 2 | yes, bundle the pages |
| **LLM-backed (`chat`, `search-semantic`)** | **2** | **no** |

So **30 of 32 views are coverable**, not 17. Only the two that need a
live model endpoint and an API key at request time are genuinely
impossible, plus the mutating surfaces (`reindex`, the webhooks,
settings writes) which are not views.

## Decision

1. **Ship a pre-computed view bundle, not the index.** A new
   `repowise export --format dashboard --out <DIR>` writes the WASM
   frontend plus a payload of endpoint responses. The frontend selects
   its data source at startup: `/api/*` when served, the bundled payload
   when static.

2. **Pre-compute the git- and ADR-derived views at export time** rather
   than declaring them absent. The exporter has the checkout; the
   viewer does not need one.

3. **`chat` and `search-semantic` are absent, not degraded.** They are
   the only two that cannot work, and a chat box that cannot answer is
   worse than no chat box. The static build omits the controls entirely
   and says why, rather than rendering something that fails on click —
   the same rule `search --mode semantic` already follows when it
   refuses `--index` instead of silently answering from elsewhere.

4. **Staleness is stamped into the payload and shown in the UI.** A
   committed dashboard is a snapshot by construction. It carries the
   `indexed_commit` it was built from and displays it, for the same
   reason ADR-0002 made staleness reporting mandatory on every portable
   index read.

5. **Complement, not replace.** `serve-dashboard` stays. The static
   build is a second data source behind one interface in
   `repowise-web`, not a second frontend — that is the whole risk of
   this change and confining it to a source-selection seam is what keeps
   the two from drifting.

## Alternatives considered

**A. Ship the portable index and compute views in the browser.** What
#383 assumed. Rejected on measurement: 6.32 MB instead of 3.23 MB, plus
reimplementing health scoring, graph queries, and community detection in
WASM — a second implementation of logic that already exists in Rust, and
the one thing guaranteed to diverge.

**B. Server-side render to static HTML**, as the pre-#59 dashboard did.
Simpler, but discards the interactivity #59/#65 deliberately built.

**C. Publish the JSON and let viewers run the server locally.** Today's
state. It does not remove the "run a process" step, which is the actual
barrier.

**D. Bundle only the 17 index-only views.** Rejected: it treats an
export-time capability as a runtime one, and it would drop hotspots and
decisions — two of the most valuable views — for no reason but the
order the code happens to compute them in.

## Consequences

**Accepted:**

- A second data-source path in `repowise-web` that must stay in step
  with the server's DTOs. Mitigated by both sides sharing the same DTO
  types, so a shape change breaks compilation rather than the page.
- ~3.2 MB committed per snapshot, and it grows with the repo. Real, and
  smaller than the alternative that was assumed.
- Two views visibly missing in the static build.

**Created:**

- **Detail-view bundling is the open size question**, and it is now
  quantified: decision details alone are 59% of the payload (171 × ~11.8
  KB) and symbol details another 25%. Bundling every detail eagerly is
  the simplest thing and the most expensive; deriving details
  client-side from the aggregate lists is cheaper and duplicates logic.
  This deserves its own decision once the seam exists — it does not
  block the seam.
- A `--workspace` static build, deferred.

**Explicitly not decided here:** whether the payload is one file or many.
Many small files let a browser fetch lazily and would make the detail
question mostly moot; one file is simpler to commit and serve. That
tradeoff should be settled against a working seam rather than in advance.

**Correction this ADR records:** #383 and #384 both name #381 as a size
prerequisite. For #383 that is now measured to be false, and the issue
should be updated rather than left to mislead whoever picks it up.
