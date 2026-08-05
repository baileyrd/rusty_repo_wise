# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- Portable, committable index: `repowise export --format index` writes a
  repo-relative, canonically-sorted, schema-versioned artifact, and
  `--index <FILE>` on `overview`/`health`/`deps`/`tour` reads one with
  mandatory staleness reporting (#378, ADR-0002).
- `repowise tour` and the `repowise-tour` crate: a deterministic,
  dependency-ordered reading path through a codebase, with `--from`,
  `--max-steps`, and `--format text|markdown` (#377).
### Changed
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
