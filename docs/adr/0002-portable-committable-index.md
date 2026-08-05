# ADR-0002: A portable, committable index artifact

Status: Proposed
Date: 2026-08-05

Tracks issue #378.

## Context

Every consumer of this port's analysis has to build the index itself.
`.repowise/` is gitignored, and the index is not portable even if it
weren't:

- `RepoIndex.root` is an absolute `PathBuf`
  (`crates/repowise-core/src/lib.rs:751`).
- `FileRecord.path` is absolute (`:734`), as is `Symbol.file`.
- `Symbol::make_id` bakes the absolute path into every symbol id:
  `format!("{}::{}@{}", file.display(), name, start_line)` (`:685`).

Measured on this repo (122 parsed files, 2,861 symbols): the index is
**8.0 MB on disk**, and the string `/home/user/rusty_repo_wise` appears
**27,884 times** in it. So the cost of indexing is paid per developer,
per machine, per CI job, forever — and anyone who only wants to *read*
the analysis (a reviewer checking a hotspot claim, a new hire, a CI job
running `workspace-conformance`) must first run a full parse.

This is already a known defect at one boundary rather than a new
discovery. The MCP `get_symbol` fix converts ids to repo-relative
precisely because absolute ids leaked the producing machine's directory
layout to every caller and, on one dense file, accounted for 59% of the
symbol list. That fix was applied at the *surface*; the storage
underneath is unchanged.

Forcing function: this is the single biggest structural gap against
[Understand-Anything](https://github.com/Egonex-AI/Understand-Anything),
whose distribution model is that `.ua/knowledge-graph.json` is committed
and shared, with the dashboard reading committed JSON and no API calls
at all.

### What the measurements say

Index composition, as a share of total JSON bytes:

| Field | Share |
| ----- | ----- |
| `calls` | 51.7% |
| `symbols` | 45.3% |
| `imports` | 1.4% |
| `field_accesses` | 1.3% |
| `path` | 0.1% |

Within `symbols`, `id` is 4.4% of the whole index and `file` 2.9% —
both dominated by the repeated absolute prefix.

Two consequences fall out of this table:

1. **Relative paths are necessary but not sufficient.** Rebasing paths
   removes roughly 750 KB of repeated prefix — real, but it does not
   turn an 8 MB artifact into a committable one.
2. **`calls` is the size story.** Dropping `calls` and `field_accesses`
   would roughly halve the artifact. That is the only lever of the right
   order of magnitude.

Determinism: two consecutive `repowise init` runs on this machine
produce byte-identical output, but that is not evidence of portability.
`discover_files` uses `ignore::WalkBuilder::build()`, whose iteration
order follows filesystem `readdir` order and is **not** guaranteed
stable across machines or filesystems. `index.files` is therefore in an
incidental order that happens to be reproducible here.

## Decision

Add a portable index artifact, distinct from the working index, via a
single conversion choke point on `RepoIndex`.

1. **Two forms, one converter.** Keep the in-memory/working index
   absolute-path as it is today. Add `RepoIndex::to_portable(&self)` and
   `RepoIndex::anchor_to(&mut self, root)`. Every path rebase lives in
   those two functions — including `Symbol.id`, which must be rewritten,
   not just `Symbol.file`. Nothing else in the workspace learns about
   the two forms.

   *Rejects* rewriting all ~215 absolute-path sites to relative
   (Alternative A below).

2. **Repo-relative throughout the artifact.** In the portable form
   `root` is `"."`, and `FileRecord.path` / `Symbol.file` / `Symbol.id`
   are repo-relative. This makes the stored form match the external
   contract `get_symbol` already exposes, rather than adding a third
   convention.

3. **An explicit `schema_version`.** A committed artifact outlives the
   binary that wrote it. A version mismatch fails loudly with "re-export
   with a newer repowise", never by silent misparse. `serde(default)`
   compatibility is the right tool for adding a field to a
   locally-regenerated index; it is the wrong tool for an artifact whose
   whole purpose is to be read by a binary that did not write it.

4. **Canonical ordering, not incidental ordering.** The exporter sorts:
   files by path, symbols by `(file, start_line, name)`, imports and
   calls by `(line, name)`. Because `readdir` order is not portable, an
   unsorted artifact would reorder itself between machines and turn
   every re-export into an unreviewable diff. Sorting is what makes
   "byte-identical across machines for the same commit" a property the
   format can actually hold, and it is cheap to guarantee at the one
   choke point.

5. **Staleness is reported, never assumed.** `RepoIndex.indexed_commit`
   already exists for this. Any command reading a committed artifact
   reports drift against the current `HEAD` the way `repowise status`
   does. A committed index will *routinely* be behind the working tree;
   presenting stale analysis as current is the one failure mode here
   that actively misleads, so it is a hard requirement rather than a
   nicety.

6. **Full artifact first; the reduced projection is a follow-up.** The
   first cut exports everything, so a committed artifact is a complete
   substitute for a local index and no command silently loses
   capability. The `calls`-dropping projection is deferred to its own
   issue, because dropping `calls` disables dead-code detection and the
   call-graph-dependent health markers — that is a capability decision,
   not a compression decision, and it deserves to be made separately
   rather than bundled into the format.

CLI surface:

```
repowise export --format index --out <FILE> [PATH]
repowise <read command> --index <FILE>
```

## Alternatives considered

**A. Rewrite storage to repo-relative throughout** (~215 sites across 19
files). This is the deeper fix: it removes the dual representation
entirely and makes the leak structurally impossible rather than
converted-away at a boundary. Rejected *for now* on risk, not on
principle — it is a breaking change to the on-disk index and to every
symbol id in flight, touching every crate, with regression risk spread
across the whole workspace and no way to stage it. The chosen design
does not foreclose it: if `to_portable` becomes the only form anyone
wants, collapsing to one representation later is a smaller step from
here than from today.

**B. Commit `.repowise/index.json` as-is.** Rejected: absolute paths make
it wrong on every machine but the one that wrote it, and 27,884 embedded
copies of one developer's home directory is a privacy leak as much as a
correctness bug.

**C. Publish indexes to a shared server or registry.** Rejected for now:
infrastructure to run, secure, and authenticate, when git is already the
artifact store every team shares. Not foreclosed.

**D. Reuse `repowise export --format json-graph`.** Rejected: that is
the dependency graph only, and deliberately a presentation format. It
carries no symbols, health, or git analytics, so it cannot back the read
commands.

**E. Make indexing fast enough that sharing doesn't matter.** Helps, and
worth doing regardless, but does not address a reviewer who wants the
answer without a checkout or a CI job with a cold cache.

## Consequences

**Accepted:**

- Two representations of the same data, and a conversion that must stay
  correct. Mitigated by confining it to one function pair with
  round-trip property tests (`to_portable` then `anchor_to` is the
  identity on every field).
- A `schema_version` this project must honour across releases: a real,
  ongoing compatibility obligation it did not have before. This is the
  main reason #378 is scoped Large and gated on an ADR.
- The first artifact is large (~8 MB here, and this is a small repo).
  Teams will need git-lfs guidance until the reduced projection lands.
  Documented rather than hidden.

**Created:**

- Follow-up: reduced read-only projection (drop `calls`/
  `field_accesses`), with an explicit statement of which commands stop
  working against it.
- Follow-up: a static, committed-index dashboard mode — Understand-
  Anything's dashboard runs with no server at all, and ours cannot yet.
- Follow-up: `--workspace` interaction, where several repos each commit
  their own artifact.

**Foreclosed:** nothing, deliberately. Alternative A stays available and
becomes cheaper, not more expensive, if this design proves out.

**Explicitly not decided here:** whether the artifact is one file or a
directory tree. One file is simpler; a split tree diffs far better in
review. This does not need to be settled before the converter and the
schema version exist, and settling it early on a guess would be worse
than settling it once there is something to measure.
