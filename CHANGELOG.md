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
- Dashboard API: `/api/health`, `/api/dead-code`,
  `/api/refactor-candidates`, and `/api/security` gained `?repo=` too —
  every federatable endpoint now honours it. `/api/search` is
  deliberately excluded: its `files` field is a bare string list, so
  labelling matches would change a type the frontend consumes (#337).
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
- Portable index records Rust/Go module paths at export time, so a
  workspace member backed by an artifact with no checkout now resolves
  cross-repo imports instead of silently contributing none (#388).
- Portable index schema v2: `CallRef.caller` is interned into a table
  instead of repeated, cutting the artifact 18.3% (7.73 MB -> 6.32 MB on
  this repo) with no capability loss. v1 artifacts are rejected with an
  actionable message and must be re-exported (#381).
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
