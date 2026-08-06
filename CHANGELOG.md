# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- Dashboard: a stylesheet. The frontend had carried class hooks
  (`empty`, `error`, `mono`, `view-nav`, `badge`, `repo-card`, ...)
  since it was written but no CSS ever existed, so every view rendered
  on browser defaults. Both colour schemes are honoured via
  `prefers-color-scheme`.
- Dashboard: a repo selector in the header, for servers started with
  `--workspace` and more than one repo. The choice lives in the URL
  hash, so it survives navigation and travels in a shared link
  (partial #337).
- Dashboard API: `/api/search` gained `?repo=` as well. It was
  previously skipped because its `files` field is a bare string list
  and a federated search can return the same relative path from two
  repos. `files` is kept exactly as it was; a new `matches` array
  carries the same paths *with* the repo that produced each one, and
  the search box renders that instead (#337).
- MCP: `get_doc_coverage` and `get_external_deps` gained `repo`,
  federating with per-entry labels — doc coverage's three counts sum
  across repos (every file lands in exactly one status), and external
  deps are deliberately **not** de-duplicated, so two repos pinning
  different versions of the same package stays visible (#337).
- MCP: `get_coupling`, `get_commits`, `get_why`, `get_answer`, and
  `get_change_risk` gained `repo` as a scope selector, completing the
  tool surface — every MCP tool now either takes `repo` or is
  workspace-level by construction (`list_repos`, `get_architecture`)
  (#337).
- MCP: `get_context`, `get_symbol`, and `get_risk` gained `repo` as a
  *scope selector*. `"all"` is rejected for anything answering about a
  single file or symbol, since there is nothing to federate; `get_risk`
  accepts it only in repo-wide mode, applying `top_n` per repo and
  grouping rather than re-ranking across repos (#337).
- Dashboard API: `/api/health`, `/api/dead-code`,
  `/api/refactor-candidates`, and `/api/security` gained `?repo=` too
  (#337).
- Dashboard API: `GET /api/overview` gained a `?repo=` query parameter
  (a named workspace repo, or `all` to federate), mirroring the MCP
  tool — federated responses add a `repos` breakdown, and an unknown
  repo or a missing `--workspace` is a 400 rather than a 500 (partial
  #337).
- MCP: `get_overview` and `get_health` gained the `repo` parameter,
  federating **per repo** (a `repos` list) rather than merging —
  overview's flat counts become workspace totals; health synthesises no
  workspace average, since a mean of means is not a mean (partial #337).
- MCP: `get_dead_code`, `get_refactor_candidates`, and
  `get_security_findings` gained the `repo` parameter (a named workspace
  repo, or `"all"` to federate across the workspace), matching
  `search_codebase` (partial #337).
- Workspace members may be backed by a committed portable index
  (`index = "..."` in the workspace TOML), so cross-repo commands no
  longer require every repo checked out and indexed. Source and
  staleness are reported per member, and a warning fires when a Rust or
  Go member without a checkout would silently contribute no edges
  (#384).
- `--index <FILE>` now works on every index-derived read command
  (`search`, `dead-code`, `refactor`, `security`, `hotspots`,
  `doc-coverage`, `decisions`, alongside the original four), with
  commands that cannot honour it rejecting the flag rather than ignoring
  it (#382).
- Portable, committable index: `repowise export --format index` writes a
  repo-relative, canonically-sorted, schema-versioned artifact, and
  `--index <FILE>` on `overview`/`health`/`deps`/`tour` reads one with
  mandatory staleness reporting (#378, ADR-0002).
- `repowise tour` and the `repowise-tour` crate: a deterministic,
  dependency-ordered reading path through a codebase, with `--from`,
  `--max-steps`, and `--format text|markdown` (#377).
### Changed
- Near-duplicate findings are a strict subset of what they were: 28%
  fewer on this repo, all of them pairs whose entire shared content was
  repo-wide boilerplate. The highest-ratio example dropped was 0.88
  between two unrelated four-line tests that shared only
  `tempfile::tempdir().unwrap()` and an `assert!` (#398).
- Portable index records Rust/Go module paths at export time, so a
  workspace member backed by an artifact with no checkout now resolves
  cross-repo imports instead of silently contributing none (#388).
- Portable index schema v2: `CallRef.caller` is interned into a table
  instead of repeated, cutting the artifact 18.3% (7.73 MB -> 6.32 MB on
  this repo) with no capability loss. v1 artifacts are rejected with an
  actionable message and must be re-exported (#381).
### Fixed
- `serve-dashboard` warms its index and health caches in the background
  at startup, so the first page load is fast too rather than paying the
  ~4-6s the caches were built to avoid. Only the server's own repo;
  workspace members stay lazy, since eagerly indexing every configured
  repo would turn startup into a long CPU burn for repos that may never
  be opened (#398).
- The dashboard server computes each repo's health report once and
  reuses it across requests, instead of `/api/files` and `/api/health`
  each recomputing one every time. Both endpoints now answer in
  milliseconds once warm: `/api/files` 3.1s -> 0.003s and
  `/api/health` 5.1s -> 0.013s (#398).
- The dashboard server parses each repo's index once and reuses it
  across requests, keyed by repo root and invalidated by the index
  file's mtime, instead of re-parsing on every request.
  `RepoIndex::load` was 2.78s on this repo, paid by every index-derived
  endpoint every time. `/api/overview` drops from 3.35s to 0.005s and
  `/api/hotspots` from 3.16s to 0.038s (#398).
- Near-duplicate detection no longer degenerates into repeated
  all-pairs comparison. Its Rabin-Karp bucketing claimed pairs "with
  nothing in common never get compared at all", but at a 3-token window
  almost every pair of functions shares *something* — on this repo the
  largest window bucket held 1308 of 2089 symbols and the scan cost
  16.9M pair-visits. Buckets too large to be discriminative no longer
  propose candidate pairs, and overlap is now scored by exact set
  intersection. `find_near_duplicates` drops from 12.4s to 2.9s, taking
  `/api/files` from 16.0s to 6.3s and `/api/health` from 19.0s to 8.5s
  (#398).
- Dashboard: picking a repo in the header changed some views and
  silently left the rest showing the server's own repo, side by side,
  with no indication — the frontend sent `?repo=` on every request but
  most endpoints never declared the parameter, so serde discarded it.
  Every index- and git-derived endpoint now honours `?repo=<name>`.
  `?repo=all` is refused with a 400 on the ones whose rows carry no
  repo label, and the frontend keeps "All repos" from reaching them.
  `POST /api/chat` takes the scope in its body, since the frontend's
  query-parameter injection only ever covered GETs (#337).
- `get_risk` collected git analytics from the MCP server's own root
  rather than from the repo being assessed. Before `repo` existed those
  were always the same path, so the bug was unreachable; adding the
  parameter would have made it reachable, and it is fixed in the same
  change (#337).
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
