# repowise Zed extension

Issue #332's first slice: registers this repo's existing MCP server
(`repowise serve`) as a [Zed](https://zed.dev/) context server, so any
Zed AI panel backed by MCP gets every existing `repowise-mcp` tool --
`search_codebase`, `get_context`, `get_risk`, `get_why`,
`get_dead_code`, `get_refactor_candidates`, `get_security_findings`, and
more. No new tools, no new analysis; this is packaging, not new
capability -- the same shape `claude-plugin/`'s own MCP registration
already takes for Claude Code.

**Retargeted from an originally-planned VS Code extension** (see the
issue): Zed extensions are written in Rust and compiled to WebAssembly,
a native fit for this workspace rather than a second TypeScript
toolchain. This also changes what's realistically portable from
upstream's own VS Code feature list -- see the issue for the full
comparison. Upstream's "health gutter/status bar, callers/ownership on
hover, refactoring CodeLens, embedded dashboards" items have no direct
Zed equivalent (Zed's extension API doesn't expose that kind of
decoration/hover/webview surface); the one thing that maps cleanly is
MCP server registration, which is what this ships.

## What it does

- **MCP server registration** (`extension.toml`'s
  `[context_servers.repowise]` + `src/lib.rs`'s
  `context_server_command`): tells Zed to spawn `repowise serve` as a
  context server. See `src/lib.rs`'s own doc comment for why no project
  path is passed explicitly -- it relies on Zed spawning the process
  with its working directory already set to the project root, the same
  convention language servers follow, since the extension API gives a
  context server no direct way to resolve a worktree ID to a filesystem
  path.

That's the entire first slice. Deliberately narrow, matching this
repo's general "clean vertical slice, extend later" pattern.

## Prerequisites

The extension invokes a `repowise` binary on `PATH` -- it doesn't
bundle or download one (a compiled Rust binary isn't portable across
platforms/architectures the way an extension bundle is). Install it
first:

```sh
cargo install --path crates/repowise-cli
```

Then run `repowise init` in the repo you want indexed. Every
`repowise-mcp` tool degrades gracefully (reports what it can, never a
raw panic) when the index is missing or stale, but a fresh index is
what makes the tools useful.

## Installing (development / local use)

Zed extensions aren't installed with a plain `cargo build` -- Zed
itself compiles the extension to `wasm32-wasip2` as part of installing
it. From Zed:

1. Open the extensions panel (`zed: extensions` in the command
   palette).
2. Choose "Install Dev Extension".
3. Point it at this directory (`zed-extension/`).

See [Zed's own developing-extensions
docs](https://zed.dev/docs/extensions/developing-extensions) for the
full local-development workflow (this repo doesn't attempt to restate
it, since it's Zed's own tooling, not repowise's).

## Scope of this first slice

- **MCP registration only.** No slash commands, no language-server
  registration, no themes -- MCP server registration was the one
  capability that maps to something this port already ships as-is.
- **No project-path argument passed to `repowise serve`** -- see
  `src/lib.rs`'s doc comment for why, and what it assumes instead.
- **Not published to Zed's extension registry.** This is a dev
  extension for now; publishing (a PR to
  [`zed-industries/extensions`](https://github.com/zed-industries/extensions))
  is a natural follow-up once the shape has been used for a while, not
  attempted here.
