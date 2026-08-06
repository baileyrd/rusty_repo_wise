# repowise Claude Code plugin

Issue #333's first slice: packages this repo's existing MCP server and a
couple of hooks as an installable Claude Code plugin, targeting Claude
Code specifically (this port's own development already runs inside it --
see the issue for why Codex/opencode support isn't attempted yet).

## What it does

- **MCP server** (`.mcp.json`): starts `repowise serve ${CLAUDE_PROJECT_DIR}`
  automatically, giving Claude every existing `repowise-mcp` tool --
  `search_codebase`, `get_context`, `get_risk`, `get_why`,
  `get_dead_code`, `get_refactor_candidates`, and more. No new tools;
  this is packaging, not new capability.
- **`SessionStart` hook**: bootstraps `.repowise/index.json` if it
  doesn't exist yet, or reports freshness if it does (never silently
  re-indexes an existing one). Cheap either way -- a first-time index
  build, or the same mtime-diffing `repowise status` already does.
- **`PreToolUse` hook** (matched to `Bash` only): routes a closed set of
  recognized commands through `repowise distill` automatically, via the
  exact same fail-open decision logic `repowise hook rewrite apply`
  already uses -- packaged for Claude Code's own hook JSON contract
  instead of a raw stdin/stdout pipe.
- **A skill** (`skills/repowise-overview/SKILL.md`) explaining which
  MCP tool or CLI command answers which kind of question, so the model
  reaches for `search_codebase` over `Grep`, `get_context` before
  editing, `get_risk` before touching a hotspot, and so on.

## Prerequisites

The plugin invokes a `repowise` binary on `PATH` -- it doesn't bundle
one (a compiled Rust binary isn't portable across platforms/
architectures the way plugin-bundled scripts are). Install it first:

```sh
cargo install --path crates/repowise-cli
```

Then run `repowise init` in the repo you want indexed, or let the
`SessionStart` hook bootstrap it automatically on first use.

## Installing (development / single-repo use)

From this repo's root:

```sh
claude --plugin-dir ./claude-plugin
```

For a persistent install without a marketplace, see [Skills-directory
plugins](https://code.claude.com/docs/en/plugins-reference#skills-directory-plugins)
in the Claude Code docs -- copy or symlink this directory under
`~/.claude/skills/` (personal) or `<project>/.claude/skills/` (project,
checked into version control) with a `.claude-plugin/plugin.json`
manifest, which this directory already has.

## Scope of this first slice

Deliberately narrow, matching this repo's general "clean vertical slice,
extend later" pattern:

- **Claude Code only** -- upstream repowise also ships Codex and
  opencode integrations; not attempted here.
- **Single-repo scoped** -- the MCP server config doesn't pass
  `--workspace`, so multi-repo workspace tools (`list_repos`,
  `get_architecture`, `get_blast_radius`, `search_codebase`'s `repo`
  parameter) aren't reachable through this plugin's default config yet.
- **No `PostToolUse` enrichment.** Upstream's own `PostToolUse` hook
  shows contextual info (related decisions, health) after
  Grep/Glob/Read/Edit/Write/Bash calls. `repowise decisions` -- the data
  source for "related decisions" -- walks full git history and isn't
  cheap enough to run synchronously after every tool call without new
  caching/plumbing this slice doesn't add. Left as documented future
  work rather than shipped slow or half-right.
- **No `AGENTS.md` generation wired into a hook** -- `repowise
  generate-claude-md` already exists as a CLI command; automating it via
  a hook is a natural follow-up, not included here.

## Multi-repo workspaces

The plugin's `.mcp.json` `args` are static, so it cannot conditionally
pass `--workspace`. Without one, every MCP tool's `repo` parameter (and
`list_repos`) answers *"requires a workspace; start the MCP server with
--workspace"* — none of the federated querying is reachable from inside
the plugin.

Export `REPOWISE_WORKSPACE` instead, pointing at your workspace TOML:

```sh
export REPOWISE_WORKSPACE=/path/to/workspace.toml
```

`repowise serve` and `repowise serve-dashboard` both pick it up when no
`--workspace` flag is given; an explicit flag always wins. A path that
doesn't exist is a hard error rather than a quiet fall back to
single-repo mode, so a typo in a shell profile can't silently disable
workspace tools. `repowise doctor` reports the variable's state either
way.
