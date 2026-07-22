# rusty_repo_wise

A Rust-native reimplementation inspired by [repowise](https://github.com/repowise-dev/repowise),
a codebase-intelligence platform that builds dependency graphs, git
analytics, auto-generated docs, architectural-decision tracking, and
deterministic code-health scoring for AI agents and developers.

This is a fresh, from-scratch Rust project — it does not share code or a
license with the original (AGPL-3.0) repowise.

## Scope of this phase

repowise is a large product with five "intelligence layers," an MCP
server, and a web dashboard. This phase builds the **core CLI plus the
dependency-graph layer**, end to end:

- Walk a codebase (respecting `.gitignore`), detect Rust and Python files.
- Parse each file with tree-sitter, extracting function/method/class/struct
  definitions, imports, and call expressions.
- Resolve imports and calls into a dependency graph (files and symbols as
  nodes; `Contains`/`Imports`/`Calls` edges), using directory-layout
  conventions (Rust's `mod`/crate-root rules, Python's package layout) —
  **not** full compiler-grade name resolution. Ambiguous or external
  references are left unresolved rather than guessed.
- Persist the index to `.repowise/index.json` and query it from the CLI.

Not yet built (out of scope for this phase): git analytics/hotspots, doc
generation, ADR mining, code-health scoring, the MCP server, and the web
dashboard. Only Rust and Python are parsed; repowise's other 14 languages
aren't implemented.

## Crates

- `repowise-core` — shared data model (`Symbol`, `FileRecord`, `RepoIndex`,
  etc.), `.gitignore`-aware file discovery, and JSON index persistence.
- `repowise-parser` — tree-sitter-based extraction for Rust and Python.
- `repowise-graph` — builds the dependency graph from a `RepoIndex` and
  answers overview/search/deps queries.
- `repowise-cli` — the `repowise` binary tying it together.

## Usage

```sh
cargo build --release

repowise init [PATH]              # build a fresh index (default PATH: .)
repowise update [PATH]             # re-index (currently a full re-index)
repowise overview [PATH]           # summary stats: languages, symbols, edges
repowise search "<query>"  [PATH]  # substring search over symbol names
repowise deps <FILE> [PATH]        # a file's resolved dependencies/dependents
```

## Testing

```sh
cargo test --workspace
```

Includes parser unit tests (function/class/import/call extraction on
inline source snippets) and graph integration tests that write real
fixture files to a temp directory to exercise Rust's `mod`/crate-root
resolution and Python's package-relative import resolution end to end.
