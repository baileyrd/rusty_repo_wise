# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
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
