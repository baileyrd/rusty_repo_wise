mod agent_md;
mod doctor;
mod export;
mod hook;
mod impacted;
mod watch;

use clap::{Parser, Subcommand};
use repowise_core::{RepoIndex, Symbol, SymbolKind};
use repowise_graph::RepoGraph;
use repowise_health::{DeadCodeCandidate, DeadCodeConfidence};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A Rust-native, self-hosted codebase intelligence CLI, inspired by
/// repowise (https://github.com/repowise-dev/repowise). Implemented so
/// far: parsing, symbol/import/call extraction, dependency-graph queries,
/// deterministic code-health scoring, git-history analytics (churn,
/// hotspots, ownership, co-change coupling), auto-generated per-file
/// documentation, architectural-decision mining, an MCP server exposing
/// a subset of these as agent-facing tools, and a live dashboard server.
#[derive(Parser)]
#[command(name = "repowise", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a fresh index of a codebase.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Re-index a codebase (currently a full re-index; incremental
    /// re-indexing is not yet implemented).
    Update {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print summary stats about the indexed codebase.
    Overview {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` -- so you can read a repo's
        /// analysis without indexing it yourself (issue #378). Drift
        /// against your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// Run a command and print a compact rendering of its output.
    /// Noise is dropped; errors, failures and summaries always survive;
    /// the command's exit code is preserved. Dropped content is stored
    /// under `.repowise/omissions/` and referenced by an inline
    /// `[repowise#<ref>]` marker -- nothing is lost, only moved. On any
    /// problem the raw output is printed unchanged.
    Distill {
        /// The command and its arguments.
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },
    /// Watch for file changes and re-index after a quiet period.
    /// Ctrl+C to stop. Drives the same deterministic re-index as
    /// `repowise update` -- no LLM generation, so it costs nothing to
    /// leave running.
    Watch {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Quiet period in milliseconds after the last change before
        /// re-indexing.
        #[arg(long, default_value_t = watch::DEFAULT_DEBOUNCE_MS)]
        debounce: u64,
        /// Print each triggering path rather than only a count.
        #[arg(long, short)]
        verbose: bool,
    },
    /// Report recurring command fumbles: the same program run twice,
    /// the first failing and a later variant succeeding. Reads the
    /// distill ledger, where exit codes are known exactly rather than
    /// inferred. Report-only unless `--write` is given.
    Corrections {
        /// How many times a fumble must recur before it's reported.
        #[arg(long, default_value_t = 2)]
        min_count: usize,
        /// Only consider records from the last N days.
        #[arg(long)]
        since_days: Option<u64>,
        /// Maintain the managed block in `.claude/CLAUDE.md`.
        #[arg(long)]
        write: bool,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Report tokens saved by `repowise distill`, from measured bytes
    /// in and out. Token counts are approximate (bytes/4, no
    /// model-specific tokenizer).
    Saved {
        /// Grouping: `program` (default) or `day`.
        #[arg(long, default_value = "program")]
        by: String,
        /// Only count records from the last N days.
        #[arg(long)]
        since_days: Option<u64>,
        /// Instead of savings, report commands the rewrite hook
        /// declined to wrap -- what's slipping past it.
        #[arg(long)]
        missed: bool,
    },
    /// Restore the output behind a `[repowise#<ref>]` omission marker
    /// left by `repowise distill`. Accepts a bare 12-hex ref or a
    /// pasted whole marker. Looks in this repo's store first, then the
    /// user-level fallback.
    Expand {
        /// A bare ref, or a pasted `[repowise#...]` marker.
        reference: String,
        /// Return only the lines matching this substring (case-
        /// insensitive). The reason output was distilled is that it was
        /// large, so grepping inside it is often more useful than
        /// dumping all of it back.
        #[arg(long, short)]
        query: Option<String>,
    },
    /// Write a managed block of codebase intelligence into
    /// `CLAUDE.md` (default `.claude/CLAUDE.md`), preserving everything
    /// outside the markers. A file with no repowise markers is
    /// appended to, never rewritten; a file whose markers are malformed
    /// is refused rather than guessed at.
    GenerateClaudeMd {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Write somewhere other than `.claude/CLAUDE.md`. Use
        /// `AGENTS.md` for agents that read that instead.
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Print the generated block to stdout and write nothing.
        #[arg(long)]
        stdout: bool,
    },
    /// Write the same managed block into `AGENTS.md` (default at the
    /// repo root) -- the cross-agent convention Codex and opencode
    /// read, where `generate-claude-md` targets Claude Code's own
    /// `.claude/CLAUDE.md`. Identical marker rules: prose outside the
    /// markers is preserved, a file with no markers is appended to, and
    /// malformed markers are refused rather than guessed at.
    GenerateAgentsMd {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Write somewhere other than `AGENTS.md`.
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Print the generated block to stdout and write nothing.
        #[arg(long)]
        stdout: bool,
    },
    /// Search the index by symbol name (default), file path, or both --
    /// case-insensitive substring match -- or by meaning with `--mode
    /// semantic`, which ranks whole files against the stored embedding
    /// index. Semantic mode needs REPOWISE_LLM_BASE_URL and an index
    /// built by `init`/`update`; without them it says so rather than
    /// falling back to substring matching, which would answer a
    /// different question than the one asked.
    Search {
        query: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        /// What to match against: `symbol` (default), `path`, `hybrid`,
        /// or `semantic`.
        #[arg(long, default_value = "symbol")]
        mode: String,
        /// Restrict to files of one role: implementation, test, config,
        /// doc, or unknown. Inferred from path conventions.
        #[arg(long)]
        kind: Option<String>,
        /// Restrict symbol hits to one kind (function, method, struct,
        /// enum, trait, class, module, mixin). Ignored in path mode.
        #[arg(long)]
        symbol_kind: Option<String>,
        /// Max results. 0 means no limit.
        #[arg(long, default_value_t = 0)]
        limit: usize,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// Show a file's resolved import dependencies and dependents.
    Deps {
        file: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// Show deterministic code-health KPIs and the lowest-scoring files.
    Health {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many of the lowest-scoring files to list.
        #[arg(long, default_value_t = 10)]
        worst: usize,
        /// A (possibly partial) TOML file of per-marker penalty
        /// overrides -- an omitted key keeps its documented default.
        /// See `repowise_health::HealthWeights` for the field names.
        #[arg(long)]
        weights: Option<PathBuf>,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// List confidence-tiered dead-code candidates: functions/methods
    /// with zero resolved in-repo callers.
    ///
    /// Even a `high`-confidence candidate is a claim about this port's
    /// own static call graph, not a runtime-safety guarantee --
    /// reflection, dynamic dispatch, and entry points are invisible to
    /// it. Review before deleting anything.
    DeadCode {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Minimum confidence tier to include: `low`, `medium`, or
        /// `high`. Mirrors the `get_dead_code` MCP tool's filter.
        #[arg(long, default_value = "low")]
        min_confidence: String,
        /// Cap the number of candidates listed; the total matching
        /// count is still reported.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// Export the generated wiki, or the dependency graph as an
    /// architecture model.
    Export {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Directory to write into.
        #[arg(long)]
        out: PathBuf,
        /// `markdown` writes the wiki page tree; `json-graph` writes the
        /// dependency graph as a single JSON Graph Format document.
        #[arg(long, value_enum, default_value_t = ExportFormat::Markdown)]
        format: ExportFormat,
        /// Write into a non-empty target directory anyway.
        #[arg(long)]
        force: bool,
    },
    /// Ingest and inspect test-coverage reports (LCOV).
    Coverage {
        #[command(subcommand)]
        action: CoverageAction,
    },
    /// Print the tests a change provably exercises, by intersecting the
    /// diff's changed lines with the per-test coverage map.
    ///
    /// An empty list is NOT evidence that nothing is affected -- the
    /// output always states which of those two it is.
    ImpactedTests {
        /// A single commit or a `base..head` range. Defaults to `HEAD`.
        revspec: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Run setup diagnostics: git availability, history depth, index
    /// presence, and which optional env-var-gated features are active.
    ///
    /// Diagnostic only -- never mutates state. A degraded-but-working
    /// setup reports `warn`, not `fail`.
    Doctor {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Manage the post-commit git hook that refreshes the index after
    /// each commit.
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
    /// Adapters for the agent-hook JSON contract shared by Claude Code
    /// and Codex (issue #333) -- distinct from `hook`/`hook rewrite`
    /// above, which are harness-agnostic (a git hook, and a
    /// stdin/stdout command rewriter any agent harness can point at).
    /// These speak the specific per-event stdin/stdout JSON a hook
    /// command is invoked with.
    ///
    /// Both hosts use the same shape -- a `hookSpecificOutput` wrapper
    /// carrying `additionalContext` for `SessionStart`/`PostToolUse`
    /// and `permissionDecision`/`updatedInput` for `PreToolUse` -- and
    /// the same event names, so one implementation serves both
    /// (verified against Codex's own hooks documentation, not assumed
    /// from the resemblance). Reachable as `agent-hook`, the
    /// host-neutral name, with `claude-hook` kept as an alias so
    /// already-installed plugin manifests keep working.
    ///
    /// Not meant to be run by hand -- installed by a plugin/config.
    #[command(name = "agent-hook", alias = "claude-hook")]
    ClaudeHook {
        #[command(subcommand)]
        action: ClaudeHookAction,
    },
    /// Report index freshness: whether an index exists, when it was
    /// written, and how much of it the working tree has moved past.
    ///
    /// Deliberately distinct from `overview`, which reports what's *in*
    /// the index -- this reports whether the index still describes the
    /// tree on disk.
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// List the individual stale/missing files rather than only
        /// counting them.
        #[arg(long)]
        verbose: bool,
    },
    /// Score the diff-shape risk of a commit or commit range.
    ///
    /// A documented fixed-weight heuristic over the shape of the diff
    /// (files, lines, subsystems, concentration, author experience) --
    /// NOT the reference repowise's ML-calibrated model. Treat the score
    /// as a rough approximation, not a calibrated probability.
    Risk {
        /// A single commit or a `base..head` range. Defaults to `HEAD`.
        revspec: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Rank files by hotspot score (git churn × cyclomatic complexity).
    Hotspots {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many of the highest-scoring files to list.
        #[arg(long, default_value_t = 15)]
        top: usize,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// Show per-author line ownership for a file, from `git blame`.
    Ownership {
        file: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Show the files that most often change alongside a given file.
    Coupled {
        file: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many co-changing files to list.
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    /// Rank the file pairs that most often change together in the same
    /// commit, across the whole repo -- unlike `coupled`, which is
    /// scoped to one file (issue #352).
    Coupling {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many pairs to list.
        #[arg(long, default_value_t = 30)]
        top: usize,
    },
    /// List the most recent commits, newest first (issue #356's
    /// dashboard Commits view). No risk score -- run `repowise risk
    /// <HASH>` for a specific commit's diff-shape score; scoring every
    /// listed commit eagerly would multiply that per-commit diff cost
    /// by however many are listed.
    Commits {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many commits to list.
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// Generate deterministic per-file documentation pages under
    /// `.repowise/wiki/`.
    Docs {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Report each indexed file's wiki-page freshness, without
    /// generating anything (issue #351): `missing` (no page yet),
    /// `fresh` (the page's embedded content hash matches the file's
    /// current content), or `stale` (the file changed since the page
    /// was last generated by `repowise docs`).
    DocCoverage {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// List every file's status rather than only the counts.
        #[arg(long)]
        verbose: bool,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// List mined architectural decisions (from docs/adr/*.md and
    /// decision-like commit messages), and which files they're linked to.
    Decisions {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Only show decisions linked to this file.
        #[arg(long)]
        for_file: Option<PathBuf>,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// Record a decision you already made, directly -- the `cli`
    /// decision source (issue #66's `cli` half; its `session`
    /// transcript-mining sibling was rejected as not planned, since this
    /// port has no agent-session-recording feature to mine from -- see
    /// issue #315). Appends to `.repowise/manual-decisions.json`; nothing
    /// recorded here is ever edited or removed by a later call.
    Decide {
        /// Short title for the decision.
        title: String,
        /// Why -- scanned for file paths/symbol names the same way an
        /// ADR file or commit message already is, so mentioning a file
        /// links this decision to it automatically.
        rationale: String,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List Dockerfile build stages and same-file `COPY --from` edges
    /// (issue #318, the prototype for #68's config/data-format tier).
    /// No wiki pages or graph integration yet -- see that issue.
    DockerStages {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List SQL/dbt objects (tables/views/functions/procedures, plus
    /// dbt models -- a whole `{{ ... }}`-templated `.sql` file, treated
    /// as one model named after its file stem) and dbt `ref()`/
    /// `source()` lineage edges (issue #317, the buildable follow-up to
    /// #67's design decision). No wiki pages or graph integration yet.
    Sql {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List OpenAPI 3.x schemas (`components.schemas`) and endpoints
    /// (one per HTTP method per path) (issue #323, the buildable
    /// follow-up to #319's design decision). Every `.yaml`/`.yml`/
    /// `.json` file in the repo is a parse candidate; only ones that
    /// actually deserialize as a valid OpenAPI 3.x document are kept --
    /// see that issue for why no `Language` variant or content-sniffing
    /// step exists for this format. No wiki pages or graph integration
    /// yet.
    Openapi {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List protobuf messages/services/RPCs (issue #324, the buildable
    /// follow-up to #319's design decision). Parsed with a pure-Rust
    /// `.proto` parser (no `protoc` binary needed); imports resolve
    /// against `.proto` files anywhere in the repo, plus bundled
    /// `google/protobuf/*` well-known types. A file whose imports don't
    /// resolve at all is silently skipped. No wiki pages or graph
    /// integration yet.
    Protobuf {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List GraphQL SDL types, plus the fields on `Query`/`Mutation`/
    /// `Subscription` root types (issue #325, the buildable follow-up
    /// to #319's design decision). Root-type detection honors an
    /// explicit `schema { query: X, ... }` block if present, else falls
    /// back to the default `Query`/`Mutation`/`Subscription` names. No
    /// wiki pages or graph integration yet.
    Graphql {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List Terraform `resource`/`module` blocks (issue #326, the
    /// buildable follow-up to #319's design decision). Parsed with
    /// generic HCL (`hcl-rs`), which has no idea `resource`/`module`
    /// are Terraform-specific -- other block types (`variable`,
    /// `output`, `provider`, `data`, `terraform`, `locals`) aren't
    /// modeled. No dependency edges between resources, no wiki pages or
    /// graph integration yet.
    Terraform {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List third-party (package-manager) dependencies declared in
    /// `Cargo.toml`, `package.json`, `composer.json`,
    /// `requirements.txt`, and `pyproject.toml`/`go.mod` (issue #353,
    /// upstream's Architecture section's Dependencies sub-view).
    /// Deliberately distinct from `deps`, which shows a single file's
    /// *internal* import dependencies -- this is external,
    /// package-manager dependencies, repo-wide. Declared, not resolved:
    /// this reports the version constraint exactly as written in the
    /// manifest, not a lockfile-resolved version -- `cargo tree`/`npm
    /// ls`/`pip list` already do full resolution per ecosystem, and
    /// duplicating that isn't this port's job. Workspace-internal path
    /// dependencies are excluded.
    ExternalDeps {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Deterministic refactor candidates: file-level import cycles, god
    /// classes, low-cohesion classes, and duplicate/near-duplicate
    /// functions -- read-only, and the only "refactoring" this port
    /// does. It names problems and where they are; it never generates a
    /// diff or writes to source (see issue #304 and its follow-up on
    /// diff generation for why that line is deliberate).
    Refactor {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Only show one kind: break-import-cycle, split-god-class,
        /// split-by-cohesion, or extract-duplicate.
        #[arg(long)]
        kind: Option<String>,
        /// Max candidates shown, 0 for unlimited. Default 20: on a real
        /// multi-crate codebase, duplicate-function candidates alone can
        /// number in the thousands (see `repowise-refactor`'s own module
        /// doc), so this defaults to capped rather than defaulting to a
        /// wall of text. Candidates are ranked strongest-first within
        /// each kind, so a cap keeps the signal.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// A guided tour: an ordered reading path through the codebase
    /// (issue #377). Ordered so nothing is introduced before what it is
    /// built out of -- foundations first, entry points last -- and
    /// selected down to the most depended-on files, since ordering every
    /// file is just the repo, shuffled.
    ///
    /// Deterministic: the ordering comes from resolved `Imports` edges
    /// and the ranking from counts already in the index, so the same
    /// commit always produces the same tour. No LLM involved.
    Tour {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Tour only this file and everything it transitively imports:
        /// "what do I have to read to understand this one file".
        #[arg(long)]
        from: Option<PathBuf>,
        /// Max stops, 0 for unlimited. Defaults to something walkable in
        /// one sitting rather than to repo coverage.
        #[arg(long, default_value_t = repowise_tour::DEFAULT_MAX_STEPS)]
        max_steps: usize,
        /// `text` (default) or `markdown`, for pasting into an
        /// onboarding doc.
        #[arg(long, default_value = "text")]
        format: String,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// List signature-based security findings (issue #360):
    /// hardcoded/leaked-secret patterns (AWS access key IDs, GitHub/
    /// Slack tokens, PEM private-key blocks, credential-shaped literal
    /// assignments). See `repowise-security`'s own module doc for why
    /// dependency-CVE checking and injection-shape detection are
    /// deliberately not covered.
    Security {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Only show findings at or above this severity: high, medium,
        /// or low. Omit for everything.
        #[arg(long)]
        min_severity: Option<String>,
        /// Read a committed portable index (`export --format index`)
        /// instead of `.repowise/index.json` (issue #378). Drift against
        /// your checkout is always reported.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// Run an MCP server over stdio exposing the agent-facing tools
    /// (get_overview, search_codebase, get_context, get_risk,
    /// get_change_risk, get_symbol, get_why, get_answer, get_dead_code,
    /// get_health, get_refactor_candidates, get_security_findings, and
    /// the workspace tools). Every response carries a `_meta` block with
    /// timing and index-staleness. Requires a prior `repowise
    /// init`/`update`.
    Serve {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Path to a workspace TOML file (see `repowise workspace-repos`
        /// and `repowise-workspace`'s own docs for the format) -- opts
        /// into the `list_repos` tool. Omit to run single-repo only.
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Add an LLM-written summary to each existing wiki page under
    /// `.repowise/wiki/` (requires a prior `repowise docs`). Opt-in:
    /// needs `REPOWISE_LLM_BASE_URL` set to an OpenAI-compatible
    /// endpoint (e.g. a self-hosted rusty_provider instance).
    Generate {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Run the live dashboard server (JSON API + optional static
    /// frontend) -- see `repowise-server`'s module doc comment for the
    /// full endpoint list.
    ServeDashboard {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: std::net::SocketAddr,
        /// Directory of the built `repowise-web` frontend (e.g.
        /// `crates/repowise-web/dist` after `trunk build`). Omit to
        /// run the JSON API alone, with no static frontend served.
        #[arg(long)]
        static_dir: Option<PathBuf>,
        /// Path to a workspace TOML file -- opts into `GET
        /// /api/workspace-repos` and the dashboard's Workspace section.
        /// Omit to run single-repo only.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Re-index automatically when HEAD moves past the indexed
        /// commit, checked every N seconds (issue #335). For a server
        /// watching a checkout that something *else* updates -- CI, a
        /// deploy script, another person's `git pull` -- where the
        /// post-commit hook and `repowise watch` (which run where edits
        /// happen) never fire and the forge may not be able to reach a
        /// webhook URL. Compares local HEAD only; never runs `git
        /// fetch`, which would mutate your repository on a timer.
        #[arg(long, value_name = "SECONDS")]
        poll: Option<u64>,
    },
    /// List every repo configured in a workspace TOML file, each with
    /// its indexed status and file count if a prior `repowise init`/
    /// `update` has run there. See `repowise-workspace`'s own docs for
    /// the file format.
    WorkspaceRepos {
        #[arg(long)]
        workspace: PathBuf,
    },
    /// Show each workspace repo's own most-coupled file pairs (from its
    /// git history), side by side. Not cross-repo co-change -- separate
    /// repos have separate git histories -- just each repo's coupling
    /// shown together in one place. See `repowise-workspace`'s own docs
    /// for the file format.
    WorkspaceCoChanges {
        #[arg(long)]
        workspace: PathBuf,
        /// How many co-changing pairs to list per repo.
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    /// Resolve imports across workspace repo boundaries: which repos
    /// depend on which others, and the individual import sites behind
    /// each dependency. Covers Rust, Python, Java, Kotlin, Scala, Go,
    /// C#, and PHP -- see `repowise-workspace`'s own docs for why every
    /// other language's cross-repo imports are left unresolved.
    WorkspaceArchitecture {
        #[arg(long)]
        workspace: PathBuf,
    },
    /// Direct (one-hop, not transitive) cross-repo importers of one
    /// file in one workspace repo: which OTHER repos' files would need
    /// review if this file's public API changed.
    WorkspaceBlastRadius {
        #[arg(long)]
        workspace: PathBuf,
        /// Name of the workspace repo the target file lives in.
        #[arg(long)]
        repo: String,
        /// Path to the file within that repo, absolute or relative to
        /// that repo's own root.
        #[arg(long)]
        file: PathBuf,
    },
    /// Report circular cross-repo dependencies (repo A imports repo B
    /// imports repo A, or a longer cycle) -- reuses exactly the edges
    /// `workspace-architecture` already computes. A workspace's
    /// repo-level dependency graph should form a DAG; a cycle is a
    /// concrete, deterministic "pattern divergence" finding.
    WorkspaceConformance {
        #[arg(long)]
        workspace: PathBuf,
        /// Emit the raw conformance report as JSON.
        #[arg(long)]
        json: bool,
        /// Treat "nothing resolvable to check" as success. Off by
        /// default: a gate that can't see anything must not report a
        /// pass, so opting into that has to be explicit.
        #[arg(long)]
        allow_unverified: bool,
    },
    /// Regex-scan every workspace repo's indexed files for HTTP
    /// producer routes (axum/Flask/FastAPI/Express-style route
    /// registration) and consumer calls (fetch/axios/requests/ureq-
    /// style), matching consumer calls against producer routes in
    /// OTHER repos. Coarse and heuristic by design -- no cross-repo
    /// symbol resolution involved, just a fixed pattern table over raw
    /// source text. See `repowise-workspace`'s own docs for the caveat.
    /// Architecture-complexity metrics over the workspace's repo-level
    /// dependency graph: propagation cost (how far a change can reach),
    /// the cyclic core (which repos form circular dependencies), and a
    /// single deterministic 1-10 score. Structural edges only --
    /// co-change is excluded deliberately.
    WorkspaceMetrics {
        #[arg(long)]
        workspace: PathBuf,
        /// Emit the raw metrics as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Explain the cross-repo contract link count that
    /// `workspace-contracts` reports: per-repo producer/consumer
    /// counts, every unmatched consumer classified by WHY it didn't
    /// match, and producers nothing in the workspace calls. Answers
    /// the question a short contract list leaves open -- whether it's
    /// an architecture finding or just an unindexed repo.
    WorkspaceDiagnostics {
        #[arg(long)]
        workspace: PathBuf,
        /// Emit the raw diagnostics as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Producer/consumer API contract matches across the workspace, plus
    /// any that used to match and stopped -- see the persisted
    /// `.repowise-workspace/contracts.json` snapshot this command reads
    /// and updates every run.
    WorkspaceContracts {
        #[arg(long)]
        workspace: PathBuf,
        /// Emit matches, unmatched consumers, and broken contracts as
        /// JSON.
        #[arg(long)]
        json: bool,
    },
}

/// What `repowise export` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ExportFormat {
    /// The generated wiki page tree, copied out verbatim.
    Markdown,
    /// The dependency graph as one JSON Graph Format document.
    JsonGraph,
    /// The full index as a portable, committable artifact (issue #378):
    /// repo-relative paths, canonical ordering, schema-versioned. Unlike
    /// `.repowise/index.json`, this is safe to commit and read on
    /// another machine.
    Index,
}

/// `repowise coverage <action>`.
#[derive(Subcommand)]
enum CoverageAction {
    /// Ingest one or more LCOV reports, merging them into any coverage
    /// already recorded.
    Add {
        /// LCOV files to ingest.
        #[arg(required = true)]
        reports: Vec<PathBuf>,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Discard previously ingested coverage instead of merging into
        /// it.
        #[arg(long)]
        replace: bool,
    },
    /// Show the per-file coverage summary and per-test map statistics.
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// How many of the least-covered files to list.
        #[arg(long, default_value_t = 10)]
        worst: usize,
    },
}

/// `repowise hook <action>`. Split from `Command` so the three actions
/// share one `hook` namespace rather than three top-level commands.
#[derive(Subcommand)]
enum HookAction {
    /// Install the post-commit hook. Refuses to overwrite a hook this
    /// tool didn't write.
    Install {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Remove the post-commit hook. Refuses to remove a hook this tool
    /// didn't write.
    Uninstall {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Report whether the hook is installed, absent, or foreign.
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Manage the command-rewrite hook, which routes recognized
    /// commands through `repowise distill` automatically.
    Rewrite {
        #[command(subcommand)]
        action: RewriteAction,
    },
}

#[derive(Subcommand)]
enum RewriteAction {
    /// Install the rewrite hook script. Refuses to overwrite a script
    /// this tool didn't write.
    Install {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Remove the rewrite hook script. Refuses to remove one this tool
    /// didn't write.
    Uninstall {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Report whether the rewrite hook is installed, and exactly which
    /// commands it will rewrite.
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Read a command line on stdin and print the command to run.
    /// Called by the installed hook; prints the input unchanged unless
    /// the command is one of a closed set of recognized shapes.
    Apply,
}

#[derive(Subcommand)]
enum ClaudeHookAction {
    /// `SessionStart`: bootstraps the index if none exists yet
    /// (`repowise init`'s own implementation), and reports freshness
    /// otherwise -- never auto-updates a stale index, the same "report,
    /// don't silently refresh behind the caller's back" stance the MCP
    /// server's own `_meta.stale_warning` already takes. Cheap enough
    /// to run synchronously because it reads the
    /// `.repowise/status.json` sidecar rather than the whole index --
    /// it measured ~3.9s in a release build when it parsed the index
    /// (twice), and ~8ms now.
    SessionStart,
    /// `PreToolUse` (matched to `Bash` only): routes the command
    /// through the exact same fail-open Distill decision logic `hook
    /// rewrite apply` already uses, wrapped in Claude Code's
    /// `PreToolUse` JSON contract instead of a raw stdin/stdout command
    /// string.
    PreToolUse,
    /// `PostToolUse` (matched to `Edit`/`Write` only): after a file is
    /// changed, reports how many other files import it, from the
    /// `.repowise/dependents.json` sidecar. Says nothing when the
    /// sidecar is missing or older than the index -- a stale blast
    /// radius reads as fact, silence reads as "no information".
    PostToolUse,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => cmd_init(&path),
        Command::Update { path } => cmd_update(&path),
        Command::Overview { path, index } => cmd_overview(&path, index.as_deref()),
        Command::Distill { command } => cmd_distill(&command),
        Command::Expand { reference, query } => cmd_expand(&reference, query.as_deref()),
        Command::Watch {
            path,
            debounce,
            verbose,
        } => cmd_watch(&path, debounce, verbose),
        Command::Corrections {
            min_count,
            since_days,
            write,
            path,
        } => cmd_corrections(min_count, since_days, write, &path),
        Command::Saved {
            by,
            since_days,
            missed,
        } => cmd_saved(&by, since_days, missed),
        Command::GenerateClaudeMd {
            path,
            output,
            stdout,
        } => cmd_generate_agent_md(
            &path,
            output.as_deref(),
            stdout,
            agent_md::DEFAULT_OUTPUT,
            "generate-claude-md",
        ),
        Command::GenerateAgentsMd {
            path,
            output,
            stdout,
        } => cmd_generate_agent_md(
            &path,
            output.as_deref(),
            stdout,
            agent_md::AGENTS_OUTPUT,
            "generate-agents-md",
        ),
        Command::Search {
            query,
            path,
            mode,
            kind,
            symbol_kind,
            limit,
            index,
        } => cmd_search(
            &query,
            &path,
            &mode,
            kind.as_deref(),
            symbol_kind.as_deref(),
            limit,
            index.as_deref(),
        ),
        Command::Deps { file, path, index } => cmd_deps(&file, &path, index.as_deref()),
        Command::Health {
            path,
            worst,
            weights,
            index,
        } => cmd_health(&path, worst, weights.as_deref(), index.as_deref()),
        Command::DeadCode {
            path,
            min_confidence,
            limit,
            index,
        } => cmd_dead_code(&path, &min_confidence, limit, index.as_deref()),
        Command::Export {
            path,
            out,
            format,
            force,
        } => cmd_export(&path, &out, format, force),
        Command::Coverage { action } => cmd_coverage(action),
        Command::ImpactedTests { revspec, path } => cmd_impacted_tests(revspec.as_deref(), &path),
        Command::Doctor { path } => cmd_doctor(&path),
        Command::Hook { action } => cmd_hook(action),
        Command::ClaudeHook { action } => cmd_claude_hook(action),
        Command::Status { path, verbose } => cmd_status(&path, verbose),
        Command::Risk { revspec, path } => cmd_risk(revspec.as_deref(), &path),
        Command::Hotspots { path, top, index } => cmd_hotspots(&path, top, index.as_deref()),
        Command::Ownership { file, path } => cmd_ownership(&file, &path),
        Command::Coupled { file, path, top } => cmd_coupled(&file, &path, top),
        Command::Coupling { path, top } => cmd_coupling(&path, top),
        Command::Commits { path, limit } => cmd_commits(&path, limit),
        Command::Docs { path } => cmd_docs(&path),
        Command::DocCoverage {
            path,
            verbose,
            index,
        } => cmd_doc_coverage(&path, verbose, index.as_deref()),
        Command::Decisions {
            path,
            for_file,
            index,
        } => cmd_decisions(&path, for_file.as_deref(), index.as_deref()),
        Command::Decide {
            title,
            rationale,
            path,
        } => cmd_decide(&path, title, rationale),
        Command::DockerStages { path } => cmd_docker_stages(&path),
        Command::Sql { path } => cmd_sql(&path),
        Command::Openapi { path } => cmd_openapi(&path),
        Command::Protobuf { path } => cmd_protobuf(&path),
        Command::Graphql { path } => cmd_graphql(&path),
        Command::Terraform { path } => cmd_terraform(&path),
        Command::ExternalDeps { path } => cmd_external_deps(&path),
        Command::Refactor {
            path,
            kind,
            limit,
            index,
        } => cmd_refactor(&path, kind.as_deref(), limit, index.as_deref()),
        Command::Tour {
            path,
            from,
            max_steps,
            format,
            index,
        } => cmd_tour(&path, from.as_deref(), max_steps, &format, index.as_deref()),
        Command::Security {
            path,
            min_severity,
            index,
        } => cmd_security(&path, min_severity.as_deref(), index.as_deref()),
        Command::Serve { path, workspace } => cmd_serve(&path, workspace),
        Command::Generate { path } => cmd_generate(&path),
        Command::ServeDashboard {
            path,
            addr,
            static_dir,
            workspace,
            poll,
        } => cmd_serve_dashboard(&path, addr, static_dir, workspace, poll),
        Command::WorkspaceMetrics { workspace, json } => cmd_workspace_metrics(&workspace, json),
        Command::WorkspaceDiagnostics { workspace, json } => {
            cmd_workspace_diagnostics(&workspace, json)
        }
        Command::WorkspaceRepos { workspace } => cmd_workspace_repos(&workspace),
        Command::WorkspaceCoChanges { workspace, top } => cmd_workspace_co_changes(&workspace, top),
        Command::WorkspaceArchitecture { workspace } => cmd_workspace_architecture(&workspace),
        Command::WorkspaceBlastRadius {
            workspace,
            repo,
            file,
        } => cmd_workspace_blast_radius(&workspace, &repo, &file),
        Command::WorkspaceConformance {
            workspace,
            json,
            allow_unverified,
        } => cmd_workspace_conformance(&workspace, json, allow_unverified),
        Command::WorkspaceContracts { workspace, json } => {
            cmd_workspace_contracts(&workspace, json)
        }
    }
}

/// Build an index and stamp it with the commit it describes.
///
/// The stamping happens here rather than in `repowise-parser` because
/// the parser has no git dependency by design. Both `init` and `update`
/// go through this, so the two can't disagree about whether an index
/// gets stamped — an unstamped index reads as "unknown commit" forever
/// downstream, and that difference should never come down to which
/// command someone happened to run.
fn build_stamped_index(root: &Path) -> anyhow::Result<RepoIndex> {
    let mut index = repowise_parser::build_index(root)?;
    index.indexed_commit = repowise_git::head_sha(&index.root);
    Ok(index)
}

/// Save an index and every sidecar derived from it (issue #333).
///
/// One function rather than a `save` call plus a `write_dependents`
/// call at each of the four places an index gets built. A site that
/// remembered one and forgot the other would leave a dependents file
/// describing a previous index -- which `load_dependents` would then
/// correctly refuse, silently costing the `PostToolUse` hook its
/// enrichment with nothing to indicate why.
///
/// The graph build costs ~0.02s on an index already in hand, and the
/// sidecar write is best effort: failing it costs a hook its
/// enrichment, never the index.
fn save_index_with_sidecars(index: &RepoIndex) -> anyhow::Result<PathBuf> {
    let saved_to = index.save(&index.root)?;
    let graph = repowise_graph::RepoGraph::build(index);
    let _ = repowise_graph::write_dependents(&index.root, index, &graph);
    Ok(saved_to)
}

fn cmd_init(path: &Path) -> anyhow::Result<()> {
    let index = build_stamped_index(path)?;
    let saved_to = save_index_with_sidecars(&index)?;
    refresh_embeddings(&index.root, &index);
    println!(
        "Indexed {} file(s) ({} other file(s) skipped) under {}",
        index.files.len(),
        index.other_files,
        index.root.display()
    );
    println!("Index written to {}", saved_to.display());
    Ok(())
}

/// Refresh the persisted embedding index after a re-index, if an LLM
/// endpoint is configured.
///
/// Opt-in and **non-fatal**: an unreachable embeddings endpoint must not
/// fail `update`, whose actual job is the code index. Reports what it
/// did rather than working silently, since this is the one part of
/// `update` that makes network calls and costs money.
fn refresh_embeddings(root: &Path, index: &RepoIndex) {
    let Some(config) = repowise_llm::LlmConfig::from_env() else {
        return;
    };
    match repowise_llm::embedding_index::refresh(root, index, &config) {
        Ok((embeddings, report)) => match embeddings.save(root) {
            Ok(_) => println!(
                "Embeddings: {} new, {} reused, {} evicted ({} file(s) covered, {} KB)",
                report.embedded,
                report.reused,
                report.evicted,
                embeddings.coverage(root, index).embedded,
                embeddings.size_bytes(root) / 1024
            ),
            Err(e) => eprintln!("Embeddings computed but could not be saved: {e}"),
        },
        Err(e) => eprintln!(
            "Embedding refresh failed, leaving the previous index in place: {e}\n\
             The code index above is unaffected; semantic search may be stale or absent."
        ),
    }
}

fn cmd_update(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let previous = RepoIndex::load(&root).ok();
    let index = build_stamped_index(&root)?;
    let saved_to = save_index_with_sidecars(&index)?;
    refresh_embeddings(&index.root, &index);
    match previous {
        Some(prev) => {
            let delta = index.files.len() as i64 - prev.files.len() as i64;
            println!(
                "Updated index: {} file(s) indexed ({:+} vs previous run)",
                index.files.len(),
                delta
            );
        }
        None => {
            println!("No previous index found; created a new one.");
            println!("{} file(s) indexed", index.files.len());
        }
    }
    println!("Index written to {}", saved_to.display());
    Ok(())
}

fn cmd_overview(path: &Path, index_file: Option<&Path>) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let graph = RepoGraph::build(&index);
    let overview = graph.overview(&index);

    println!("Repowise overview for {}", index.root.display());
    println!(
        "  {} indexed file(s), {} other file(s)",
        overview.file_count, overview.other_file_count
    );
    println!("  {} total lines", overview.total_lines);
    println!("  By language:");
    for (lang, count) in &overview.by_language {
        println!("    {lang:<10} {count}");
    }
    println!("  Symbols:");
    for (kind, count) in &overview.symbol_counts {
        println!("    {kind:<10} {count}");
    }
    println!(
        "  Edges: {} import(s), {} call(s) ({} unresolved import(s), {} unresolved call(s))",
        overview.import_edges,
        overview.call_edges,
        overview.unresolved_imports,
        overview.unresolved_calls
    );
    if !overview.most_depended_on.is_empty() {
        println!("  Most depended-on files:");
        for (file, count) in &overview.most_depended_on {
            println!("    {:<4} {}", count, display_path(file, &index.root));
        }
    }
    Ok(())
}

/// Build the store for the current directory, plus the user-level
/// fallback store if one would be different.
///
/// Two stores, checked in order, because `distill` writes to whichever
/// applies: inside a repo, alongside the index; outside one, under
/// `$HOME`. An `expand` that only checked one would fail to find refs
/// its own `distill` had just written.
fn distill_stores() -> anyhow::Result<Vec<repowise_distill::Store>> {
    let cwd = std::env::current_dir()?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let repo_dir = repowise_distill::store::store_dir(&cwd, home.as_deref());

    let mut stores = vec![repowise_distill::Store::open(repo_dir.clone())];
    if let Some(home) = &home {
        let fallback = home
            .join(".repowise")
            .join(repowise_distill::store::STORE_DIR);
        if fallback != repo_dir {
            stores.push(repowise_distill::Store::open(fallback));
        }
    }
    Ok(stores)
}

/// Watch `root` and re-index after each quiet period.
///
/// Runs until interrupted. The debounce is a *quiet period*, not a
/// fixed interval: the timer restarts on every accepted event, so a
/// burst of saves produces one re-index after the burst rather than one
/// per save.
fn cmd_watch(path: &Path, debounce_ms: u64, verbose: bool) -> anyhow::Result<()> {
    use notify::{RecursiveMode, Watcher};
    use std::sync::mpsc;

    let root = path.canonicalize()?;
    let debounce = watch::debounce_duration(debounce_ms);

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        // Send both events and errors down one channel so a backend
        // failure surfaces in the loop rather than being dropped in a
        // callback nobody reads.
        let _ = tx.send(res);
    })?;
    watcher.watch(&root, RecursiveMode::Recursive)?;

    println!(
        "Watching {} (debounce {}ms). Ctrl+C to stop.",
        root.display(),
        debounce_ms
    );

    let mut pending: Vec<PathBuf> = Vec::new();
    loop {
        // Block indefinitely when idle; once something is pending, wait
        // only for the rest of the quiet period.
        let received = if pending.is_empty() {
            rx.recv().map_err(|_| ()).map(Some)
        } else {
            match rx.recv_timeout(debounce) {
                Ok(v) => Ok(Some(v)),
                Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
            }
        };

        match received {
            // Channel closed: the watcher thread is gone. The index
            // would silently stop updating while this process still
            // looks alive, so this is fatal and loud.
            Err(()) => {
                anyhow::bail!(
                    "the filesystem watcher stopped unexpectedly -- the index is no \
                     longer being updated. Re-run `repowise watch`, or use `repowise \
                     hook install` for per-commit updates instead."
                );
            }
            Ok(Some(Ok(event))) => {
                // Kind first: a re-index reads every file, and reads
                // arrive as events on ordinary source paths that no
                // path filter can distinguish from real edits.
                if !watch::is_content_change(&event.kind) {
                    continue;
                }
                for p in event.paths {
                    if watch::should_reindex(&p, &root) {
                        if verbose {
                            println!("  changed: {}", p.display());
                        }
                        pending.push(p);
                    }
                }
            }
            Ok(Some(Err(e))) => {
                // A watcher error is not a warning. On Linux this is
                // usually an exhausted inotify limit, after which
                // changes are missed silently -- exactly when the user
                // most believes the index is current.
                anyhow::bail!(
                    "filesystem watch error: {e}\n\
                     Changes may be going unnoticed, so the index cannot be trusted \
                     to be current. On Linux this is often an exhausted inotify watch \
                     limit (see /proc/sys/fs/inotify/max_user_watches)."
                );
            }
            // Quiet period elapsed with something pending.
            Ok(None) => {
                let count = pending.len();
                pending.clear();
                match build_stamped_index(&root) {
                    Ok(index) => match save_index_with_sidecars(&index) {
                        Ok(_) => println!(
                            "re-indexed after {count} change(s): {} file(s)",
                            index.files.len()
                        ),
                        // A failed write is reported and the loop
                        // continues: a transient error shouldn't end a
                        // long-running watch, but it must not look like
                        // a success either.
                        Err(e) => eprintln!("re-index succeeded but writing the index failed: {e}"),
                    },
                    Err(e) => eprintln!("re-index failed: {e}"),
                }
            }
        }
    }
}

/// Render the fumble report.
///
/// `observed` is the number of runs with a known exit status. It leads
/// the report because a thin result is ambiguous without it: few
/// findings could mean few fumbles, or it could mean almost nothing was
/// watched. Those are different claims and the reader can't tell them
/// apart from a count of findings alone.
fn render_corrections(
    fumbles: &[repowise_distill::corrections::Fumble],
    observed: usize,
    min_count: usize,
) -> String {
    if observed == 0 {
        return "No command runs observed.\n\
                This reads the `repowise distill` ledger, so it only sees commands run\n\
                through `repowise distill` or the rewrite hook (`repowise hook rewrite\n\
                install`). Nothing observed is not the same as no fumbles.\n"
            .to_string();
    }

    let mut out = String::new();
    if fumbles.is_empty() {
        out.push_str(&format!(
            "No recurring fumbles across {observed} observed run(s) \
             (threshold: {min_count} occurrence(s)).\n"
        ));
        out.push_str(
            "\nOnly commands run through distill are observed, and only exit codes it saw\n\
             directly are counted -- nothing here is inferred from output text.\n",
        );
        return out;
    }

    out.push_str(&format!(
        "Recurring command fumbles ({observed} observed run(s), threshold {min_count}):\n\n"
    ));
    out.push_str(&format!(
        "  {:<20} {:>8}  {}\n",
        "program", "times", "failing exit code(s)"
    ));
    for f in fumbles {
        let codes: Vec<String> = f.exit_codes.iter().map(|c| c.to_string()).collect();
        out.push_str(&format!(
            "  {:<20} {:>8}  {}\n",
            f.program,
            f.count,
            codes.join(", ")
        ));
    }
    out.push_str(
        "\nEach row is a run that exited nonzero followed shortly by a successful run of\n\
         the same program -- a command that took more than one attempt to get right.\n\
         A single repeated exit code suggests one recurring mistake; several different\n\
         codes suggest the program is failing for varied reasons.\n",
    );
    out
}

/// The managed-block body written by `--write`.
///
/// Programs and counts only -- deliberately no argv. Commands carry
/// secrets (tokens in URLs, credentials in flags), and this text lands
/// in a file that gets committed. A command *shape* is the useful part
/// anyway.
fn corrections_block_body(fumbles: &[repowise_distill::corrections::Fumble]) -> String {
    let mut out = String::from("### Known command corrections\n\n");
    out.push_str(
        "Commands that have repeatedly needed more than one attempt in this repo.\n\
         Derived from exit codes observed by `repowise distill`; program names only,\n\
         never full command lines.\n\n",
    );
    for f in fumbles {
        out.push_str(&format!(
            "- `{}` -- needed a second attempt {} time(s)\n",
            f.program, f.count
        ));
    }
    out
}

fn cmd_corrections(
    min_count: usize,
    since_days: Option<u64>,
    write: bool,
    path: &Path,
) -> anyhow::Result<()> {
    let stores = distill_stores()?;
    let mut records: Vec<repowise_distill::ledger::Record> = stores
        .iter()
        .flat_map(|s| repowise_distill::ledger::read(s.dir()))
        .collect();
    records.sort_by_key(|r| r.at);

    if let Some(days) = since_days {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(days * 86_400);
        records.retain(|r| r.at >= cutoff);
    }

    let observed = repowise_distill::corrections::observed_runs(&records);
    let fumbles = repowise_distill::corrections::detect(&records, min_count);
    print!("{}", render_corrections(&fumbles, observed, min_count));

    if !write {
        return Ok(());
    }
    if fumbles.is_empty() {
        println!("\nNothing to write.");
        return Ok(());
    }

    // Reuses agent_md's marker discipline rather than growing a second
    // one: a file a human edits must never lose hand-written content to
    // a regeneration.
    let root = path.canonicalize()?;
    let target = root.join(agent_md::DEFAULT_OUTPUT);
    let block = format!(
        "{}\n\n{}\n{}",
        agent_md::BEGIN_MARKER,
        corrections_block_body(&fumbles),
        agent_md::END_MARKER
    );
    let existing = std::fs::read_to_string(&target).ok();
    let (content, action) =
        agent_md::splice(existing.as_deref(), &block).map_err(|e| anyhow::anyhow!(e))?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, content)?;
    println!("\n{} {}", action.label(), target.display());
    Ok(())
}

/// Render the savings report.
///
/// Split out so the wording -- especially the caveats, which are most
/// of what keeps this number honest -- is testable without a ledger on
/// disk.
fn render_saved(records: &[repowise_distill::ledger::Record], by: &str) -> String {
    use repowise_distill::ledger::{approx_tokens, Kind};
    use std::collections::BTreeMap;

    let distilled: Vec<_> = records
        .iter()
        .filter(|r| r.kind == Kind::Distilled)
        .collect();

    if distilled.is_empty() {
        let mut out = String::from(
            "No distillations recorded yet.\n\
             This counts only what `repowise distill` actually ran -- \
             if you haven't wrapped a command\n\
             (or installed the rewrite hook), there is nothing to measure.\n",
        );
        // A ledger can hold MCP records and no distillations. Suppressing
        // the estimate here would hide real data behind an unrelated
        // empty state.
        out.push_str(&render_mcp_estimate(records));
        return out;
    }

    let raw: usize = distilled.iter().map(|r| r.raw_bytes).sum();
    let kept: usize = distilled.iter().map(|r| r.kept_bytes).sum();
    let saved: usize = distilled.iter().map(|r| r.saved_bytes()).sum();

    let mut groups: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in &distilled {
        let key = if by == "day" {
            // Whole days since the epoch -- enough to bucket by date
            // without pulling in a calendar library for a rollup.
            format!("day {}", r.at / 86_400)
        } else {
            r.program.clone()
        };
        let entry = groups.entry(key).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.saved_bytes();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "Distillation savings ({} run(s))\n\n",
        distilled.len()
    ));
    out.push_str(&format!(
        "  {:<24} {:>8} {:>14}\n",
        "group", "runs", "~tokens saved"
    ));
    for (key, (runs, bytes)) in &groups {
        out.push_str(&format!(
            "  {:<24} {:>8} {:>14}\n",
            key,
            runs,
            approx_tokens(*bytes)
        ));
    }
    out.push_str(&format!(
        "\n  raw output:      {:>10} bytes (~{} tokens)\n  printed:         {:>10} bytes (~{} tokens)\n  saved:           {:>10} bytes (~{} tokens)\n",
        raw,
        approx_tokens(raw),
        kept,
        approx_tokens(kept),
        saved,
        approx_tokens(saved)
    ));

    out.push_str(
        "\nEvery figure above is measured: bytes that went into a distillation and\n\
         bytes that came out, for commands that actually ran. Token counts are\n\
         approximate -- bytes/4, with no model-specific tokenizer -- so treat them\n\
         as an order of magnitude, not an invoice.\n",
    );
    out.push_str(&render_mcp_estimate(records));
    out
}

/// The MCP block: a **modelled** estimate, reported separately from
/// everything above it.
///
/// Kept in its own function and its own section on purpose. The measured
/// totals are bytes that actually moved; this is a counterfactual, and
/// the two must never appear as one number. `Record::is_measured` is the
/// structural half of that guarantee -- this is the presentational half:
/// its own heading, its own totals, and the model stated inline so the
/// figure can be argued with rather than just believed.
fn render_mcp_estimate(records: &[repowise_distill::ledger::Record]) -> String {
    use repowise_distill::ledger::{approx_tokens, Kind};
    use std::collections::BTreeMap;

    let mcp: Vec<_> = records
        .iter()
        .filter(|r| r.kind == Kind::McpResponse)
        .collect();

    if mcp.is_empty() {
        return "\nMCP tool responses: none recorded.\n\
                Recorded only for tools whose covered-file set is unambiguous \
                (`get_context`,\n\
                `get_symbol`). Nothing recorded means the server hasn't served \
                those, not that\n\
                they saved nothing.\n"
            .to_string();
    }

    let baseline: usize = mcp.iter().map(|r| r.raw_bytes).sum();
    let responses: usize = mcp.iter().map(|r| r.kept_bytes).sum();
    let avoided: usize = mcp.iter().map(|r| r.saved_bytes()).sum();

    let mut groups: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in &mcp {
        let entry = groups.entry(r.program.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.saved_bytes();
    }

    // Calls where the curated answer was BIGGER than the files it
    // described. `saved_bytes` saturates at zero, so without counting
    // these separately the report would quietly round a loss up to
    // "saved nothing" -- flattering the very number that most needs to
    // be trustworthy.
    let costlier: Vec<_> = mcp.iter().filter(|r| r.kept_bytes > r.raw_bytes).collect();
    let overhead: usize = costlier
        .iter()
        .map(|r| r.kept_bytes.saturating_sub(r.raw_bytes))
        .sum();

    let mut out = String::from("\nMCP tool responses -- ESTIMATED, not measured\n\n");
    out.push_str(&format!(
        "  {:<24} {:>8} {:>18}\n",
        "tool", "calls", "~tokens avoided"
    ));
    for (tool, (calls, bytes)) in &groups {
        out.push_str(&format!(
            "  {:<24} {:>8} {:>18}\n",
            tool,
            calls,
            approx_tokens(*bytes)
        ));
    }
    out.push_str(&format!(
        "\n  modelled baseline: {:>10} bytes (~{} tokens)\n  \
         actual responses:  {:>10} bytes (~{} tokens)\n  \
         estimated avoided: {:>10} bytes (~{} tokens)\n",
        baseline,
        approx_tokens(baseline),
        responses,
        approx_tokens(responses),
        avoided,
        approx_tokens(avoided)
    ));

    if !costlier.is_empty() {
        out.push_str(&format!(
            "\n  {} of {} call(s) returned MORE than the files they described,\n  \
             by {} bytes (~{} tokens) in total. Those are counted as zero avoided,\n  \
             not as a negative -- but they are a real cost, not a saving.\n",
            costlier.len(),
            mcp.len(),
            overhead,
            approx_tokens(overhead)
        ));
    }

    out.push_str(
        "\n  The model: baseline = the total on-disk size of the files each answer\n\
         \x20 covered, i.e. what reading them instead would have cost. Real file sizes,\n\
         \x20 but still a counterfactual -- the caller might have read only part of a\n\
         \x20 file, or might have read more, or might not have looked at all. Treat this\n\
         \x20 as an upper bound on a plausible alternative, not as bytes anyone saved.\n\
         \x20 It is deliberately NOT added to the measured totals above.\n\
         \x20 Recorded only where the covered-file set is unambiguous: `get_overview`\n\
         \x20 and `search_codebase` answer about the repo rather than a knowable set of\n\
         \x20 files, so no baseline is claimed for them.\n",
    );
    out
}

/// Render the `--missed` report: what the rewrite hook let past.
fn render_missed(records: &[repowise_distill::ledger::Record]) -> String {
    use repowise_distill::ledger::Kind;
    use std::collections::BTreeMap;

    let skipped: Vec<_> = records.iter().filter(|r| r.kind == Kind::Skipped).collect();
    if skipped.is_empty() {
        return "No skipped commands recorded.\n\
                This is populated by the rewrite hook -- if it isn't installed \
                (`repowise hook rewrite install`),\n\
                nothing is being observed, which is not the same as nothing being missed.\n"
            .to_string();
    }

    let mut groups: BTreeMap<(String, String), usize> = BTreeMap::new();
    for r in &skipped {
        *groups
            .entry((r.program.clone(), r.detail.clone()))
            .or_insert(0) += 1;
    }
    let mut rows: Vec<_> = groups.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));

    let mut out = String::from("Commands the rewrite hook declined to wrap:\n\n");
    out.push_str(&format!(
        "  {:<20} {:<18} {:>6}\n",
        "program", "reason", "times"
    ));
    for ((program, reason), count) in rows {
        out.push_str(&format!("  {program:<20} {reason:<18} {count:>6}\n"));
    }
    out.push_str(
        "\n`not-rewritable` means the program isn't in the closed set the hook understands --\n\
         a candidate for widening it. `shell-syntax` is deliberate and will never be\n\
         rewritten, however often it appears.\n",
    );
    out
}

fn cmd_saved(by: &str, since_days: Option<u64>, missed: bool) -> anyhow::Result<()> {
    if by != "program" && by != "day" {
        anyhow::bail!("--by must be `program` or `day`, got {by:?}");
    }
    let stores = distill_stores()?;
    let mut records: Vec<repowise_distill::ledger::Record> = stores
        .iter()
        .flat_map(|s| repowise_distill::ledger::read(s.dir()))
        .collect();

    if let Some(days) = since_days {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(days * 86_400);
        records.retain(|r| r.at >= cutoff);
    }

    if missed {
        print!("{}", render_missed(&records));
    } else {
        print!("{}", render_saved(&records, by));
    }
    Ok(())
}

fn cmd_expand(reference: &str, query: Option<&str>) -> anyhow::Result<()> {
    let Some(parsed) = repowise_distill::parse_ref(reference) else {
        anyhow::bail!(
            "{reference:?} isn't a repowise omission ref. Expected 12 hex digits, or a \
             pasted marker like `[repowise#a1b2c3d4e5f6: ...]`."
        );
    };

    let stores = distill_stores()?;
    let mut content = None;
    for store in &stores {
        if let Ok(found) = store.get(&parsed) {
            content = Some(found);
            break;
        }
    }

    let Some(content) = content else {
        // Distinguishing these matters: the store's size cap and TTL
        // mean a well-formed ref genuinely can stop existing. Reporting
        // that as a plain "not found" would send someone looking for a
        // typo they didn't make.
        anyhow::bail!(
            "no stored output for ref {parsed}. It was either never written, or the \
             omission store evicted it -- entries expire after {} days or when the \
             store passes its size cap. Searched: {}",
            repowise_distill::store::TTL.as_secs() / 86_400,
            stores
                .iter()
                .map(|s| s.dir().display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    };

    let Some(query) = query else {
        print!("{content}");
        if !content.ends_with('\n') {
            println!();
        }
        return Ok(());
    };

    let needle = query.to_lowercase();
    let matching: Vec<&str> = content
        .lines()
        .filter(|l| l.to_lowercase().contains(&needle))
        .collect();

    if matching.is_empty() {
        // Not the same as an empty ref, and saying so keeps someone from
        // concluding the stored output was blank.
        println!(
            "No lines matching {query:?} in ref {parsed} ({} line(s) stored).",
            content.lines().count()
        );
        return Ok(());
    }
    for line in matching {
        println!("{line}");
    }
    Ok(())
}

/// Run a command and print a distilled rendering of its output.
///
/// Three properties this has to preserve, all of which make it a
/// drop-in wrapper rather than a thing you have to think about:
///
/// - **The exit code passes through**, so wrapping a command in a
///   script doesn't change that script's behavior.
/// - **stdout and stderr stay separate.** Interleaving them into one
///   stream to filter would corrupt output for anything that pipes.
///   They're captured, distilled, and written back to their own
///   streams.
/// - **Any problem prints raw.** Handled inside `repowise_distill`,
///   but the same rule applies to the spawn itself: a command we can't
///   run is an error about *that*, not a distillation failure.
fn cmd_distill(command: &[String]) -> anyhow::Result<()> {
    use std::io::Write;

    let (program, args) = command
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("no command given"))?;

    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run {program:?}: {e}"))?;

    let cwd = std::env::current_dir()?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let store =
        repowise_distill::Store::open(repowise_distill::store::store_dir(&cwd, home.as_deref()));

    let stdout_raw = String::from_utf8_lossy(&output.stdout);
    let stderr_raw = String::from_utf8_lossy(&output.stderr);

    let rendered_out = repowise_distill::render(&stdout_raw, &store);
    let rendered_err = repowise_distill::render(&stderr_raw, &store);

    // Measured accounting: bytes actually in, bytes actually out, for a
    // command that actually ran. Recorded only when something was
    // distilled -- a pass-through saved nothing and logging it as a
    // zero-saving row would dilute the report with non-events.
    let raw_bytes = stdout_raw.len() + stderr_raw.len();
    let kept_bytes = rendered_out.text.len() + rendered_err.text.len();
    let exit_code = output.status.code().unwrap_or(1);
    let name = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);
    if rendered_out.reference.is_some() || rendered_err.reference.is_some() {
        repowise_distill::ledger::record_distilled(
            store.dir(),
            name,
            raw_bytes,
            kept_bytes,
            exit_code,
        );
    } else {
        // Recorded even though it saved nothing: the *succeeding* half
        // of a fumble pair is usually short output that passed straight
        // through, and without it every fumble would look uncorrected.
        // `Ran` keeps these out of the savings totals.
        repowise_distill::ledger::record_ran(store.dir(), name, raw_bytes, exit_code);
    }

    if !rendered_out.text.is_empty() {
        print!("{}", rendered_out.text);
        if !rendered_out.text.ends_with('\n') {
            println!();
        }
    }
    if !rendered_err.text.is_empty() {
        let mut err = std::io::stderr();
        write!(err, "{}", rendered_err.text)?;
        if !rendered_err.text.ends_with('\n') {
            writeln!(err)?;
        }
    }

    // Exit-code preservation. `std::process::exit` rather than
    // returning Err: an anyhow error would print its own message and
    // use its own code, which would make this wrapper visible to the
    // script wrapping it.
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

/// Backs both `generate-claude-md` and `generate-agents-md` (issue
/// #333). The two differ only in where they write by default and in the
/// command they name inside the block -- the managed-marker rules,
/// content, and refusal behaviour are deliberately identical, so a repo
/// keeping both files can't have them drift.
fn cmd_generate_agent_md(
    path: &Path,
    output: Option<&Path>,
    stdout: bool,
    default_output: &str,
    generator: &str,
) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);
    let block = agent_md::render_block(&index, &graph, &root, generator);

    if stdout {
        println!("{block}");
        return Ok(());
    }

    let target = match output {
        Some(p) if p.is_absolute() => p.to_path_buf(),
        Some(p) => root.join(p),
        None => root.join(default_output),
    };

    let existing = std::fs::read_to_string(&target).ok();
    let (content, action) =
        agent_md::splice(existing.as_deref(), &block).map_err(|e| anyhow::anyhow!(e))?;

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, content)?;
    println!("{} {}", action.label(), target.display());
    Ok(())
}

/// Semantic search over the persisted embedding index.
///
/// Embeds only the query -- one short call -- against vectors stored by
/// `update`. Reports coverage, because a search over part of a repo that
/// presents itself as a search over the repo is the failure this feature
/// could most easily introduce.
fn cmd_search_semantic(query: &str, path: &Path, limit: usize) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let config = repowise_llm::LlmConfig::from_env();

    let (hits, coverage) =
        match repowise_llm::embedding_index::search(&root, &index, query, config.as_ref()) {
            Ok(found) => found,
            // Refused rather than degraded: falling back to substring
            // matching would answer a different question than the one
            // asked, which is the same rule the mode parser follows.
            Err(reason) => anyhow::bail!("{}", reason.explain()),
        };

    if !coverage.is_complete() {
        println!(
            "NOTE: {} of {} file(s) have embeddings{}. Files without one cannot be \
             ranked and are absent from these results -- run `repowise update` to \
             embed them.",
            coverage.embedded,
            coverage.total,
            coverage
                .percent()
                .map(|p| format!(" ({p:.0}%)"))
                .unwrap_or_default()
        );
    }

    if hits.is_empty() {
        println!("No embedded files to rank for {query:?}.");
        return Ok(());
    }

    let shown = if limit == 0 {
        hits.len()
    } else {
        limit.min(hits.len())
    };
    for hit in hits.iter().take(shown) {
        println!(
            "{:<8} {:.3}  {}",
            "file",
            hit.similarity,
            display_path(&hit.file, &index.root)
        );
    }
    if shown < hits.len() {
        println!("... {shown} of {} shown (--limit {limit})", hits.len());
    }
    println!(
        "\nRanked by embedding similarity over per-file symbol summaries, not over file \
         contents -- this port's index stores structure, not source."
    );
    Ok(())
}

/// Describe the filters that were applied, for the no-results message.
///
/// An empty result is ambiguous: "nothing in this repo matches" and
/// "your filters excluded everything" look identical, and the second is
/// the far more likely explanation once flags are in play. Echoing the
/// active filters back is what lets someone tell which they're looking
/// at without re-running the command to find out.
fn active_filters(
    mode: repowise_graph::SearchMode,
    kind: Option<repowise_graph::FileKind>,
    symbol_kind: Option<SymbolKind>,
) -> String {
    let mut parts = vec![format!("mode={}", mode.label())];
    if let Some(k) = kind {
        parts.push(format!("kind={}", k.label()));
    }
    if let Some(k) = symbol_kind {
        parts.push(format!("symbol_kind={}", k.label()));
    }
    parts.join(", ")
}

fn cmd_search(
    query: &str,
    path: &Path,
    mode: &str,
    kind: Option<&str>,
    symbol_kind: Option<&str>,
    limit: usize,
    index_file: Option<&Path>,
) -> anyhow::Result<()> {
    let mode = repowise_graph::SearchMode::parse(mode).map_err(|e| anyhow::anyhow!(e))?;
    // Semantic is handled separately: it ranks whole files by embedding
    // similarity rather than matching symbol names, so it shares none of
    // the filtering below. Dispatching on `is_lexical` rather than on a
    // list of variants means a future mode that also isn't substring
    // matching can't fall through into filters that don't fit it.
    if !mode.is_lexical() {
        // Rejected rather than ignored, matching this command's existing
        // rule for filters it can't honour: semantic mode ranks by an
        // embedding index that lives under the *local* `.repowise/` and
        // is tied to the root that built it. A portable artifact carries
        // no embeddings, so accepting `--index` here would silently
        // answer from a different source than the caller named.
        if index_file.is_some() {
            anyhow::bail!(
                "--index is not supported with --mode semantic: embeddings live in the \
                 local .repowise/ and aren't part of a portable index. Use a lexical \
                 mode (symbol/path/hybrid), or run `repowise update` to embed them."
            );
        }
        return cmd_search_semantic(query, path, limit);
    }
    let kind = kind
        .map(repowise_graph::FileKind::parse)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;
    let symbol_kind = symbol_kind
        .map(repowise_graph::parse_symbol_kind)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;

    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let graph = RepoGraph::build(&index);

    // A file passes the `--kind` filter, or there is no filter.
    let file_allowed = |file: &Path| -> bool {
        let Some(want) = kind else { return true };
        index
            .files
            .iter()
            .find(|f| f.path == file)
            .map(|f| repowise_graph::classify(f, &index.root) == want)
            .unwrap_or(false)
    };

    let mut symbol_hits: Vec<&Symbol> = Vec::new();
    if matches!(
        mode,
        repowise_graph::SearchMode::Symbol | repowise_graph::SearchMode::Hybrid
    ) {
        symbol_hits = graph
            .search(query)
            .into_iter()
            .filter(|s| symbol_kind.is_none_or(|k| s.kind == k))
            .filter(|s| file_allowed(&s.file))
            .collect();
        symbol_hits.sort_by(|a, b| a.name.cmp(&b.name).then(a.file.cmp(&b.file)));
    }

    let mut path_hits: Vec<&Path> = Vec::new();
    if matches!(
        mode,
        repowise_graph::SearchMode::Path | repowise_graph::SearchMode::Hybrid
    ) {
        path_hits = index
            .files
            .iter()
            .filter(|f| repowise_graph::path_matches(&f.path, &index.root, query))
            .filter(|f| kind.is_none_or(|k| repowise_graph::classify(f, &index.root) == k))
            .map(|f| f.path.as_path())
            .collect();
        path_hits.sort();
    }

    let total = symbol_hits.len() + path_hits.len();
    if total == 0 {
        println!(
            "No matches for {query:?} ({})",
            active_filters(mode, kind, symbol_kind)
        );
        return Ok(());
    }

    let shown = if limit == 0 { total } else { limit.min(total) };
    let mut printed = 0usize;

    for sym in &symbol_hits {
        if printed >= shown {
            break;
        }
        println!(
            "{:<8} {:<30} {}:{}",
            sym.kind.label(),
            sym.name,
            display_path(&sym.file, &index.root),
            sym.start_line
        );
        printed += 1;
    }
    for file in &path_hits {
        if printed >= shown {
            break;
        }
        println!("{:<8} {}", "file", display_path(file, &index.root));
        printed += 1;
    }

    if printed < total {
        println!("... {} of {total} shown (--limit {limit})", printed);
    }
    Ok(())
}

fn cmd_deps(file: &Path, path: &Path, index_file: Option<&Path>) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let graph = RepoGraph::build(&index);

    let target = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let target = target.canonicalize().unwrap_or(target);

    let deps = graph.dependencies_of(&target);
    let dependents = graph.dependents_of(&target);

    println!("{}", display_path(&target, &index.root));
    println!("  depends on ({}):", deps.len());
    for d in &deps {
        println!("    {}", display_path(d, &index.root));
    }
    println!("  depended on by ({}):", dependents.len());
    for d in &dependents {
        println!("    {}", display_path(d, &index.root));
    }
    Ok(())
}

/// Upper bound on how many files count as "hot" when scoring
/// `hot_path_sync_io` (issue #186).
const HOT_PATH_MAX_FILES: usize = 10;

/// ...and the fraction of the repo that bound is additionally capped
/// to. Without this, a small repo has *every* file in its top 10, which
/// silently turns `hot_path_sync_io` into "any sync I/O anywhere" and
/// throws away the empirical half of the signal that justifies the
/// marker. A repo of 8 files gets a top 2; the cap only stops binding
/// once there are 40+ files.
const HOT_PATH_REPO_FRACTION: usize = 4;

/// The hottest files by hotspot score: the top
/// `HOT_PATH_MAX_FILES`, further capped to a
/// `1/HOT_PATH_REPO_FRACTION` slice of the repo, and never including a
/// file scoring zero (no churn or no complexity is not a hot path by
/// any definition). A relative rank rather than an absolute score
/// threshold, because hotspot scores are churn x complexity and so
/// aren't comparable between repos -- "the files this repo churns
/// hardest" travels; "score above 500" doesn't.
///
/// Returns an empty set when git history isn't available (no repo, a
/// shallow clone, git missing). Failing soft is deliberate:
/// `hot_path_sync_io` is the only marker that needs this, and losing one
/// marker is a much better outcome than refusing to score at all.
fn hot_path_files(
    index: &RepoIndex,
    analytics: &repowise_git::GitAnalytics,
) -> std::collections::HashSet<PathBuf> {
    let limit = HOT_PATH_MAX_FILES.min((index.files.len() / HOT_PATH_REPO_FRACTION).max(1));
    repowise_git::hotspots(index, analytics)
        .into_iter()
        .filter(|h| h.score > 0)
        .take(limit)
        .map(|h| h.file)
        .collect()
}

fn cmd_health(
    path: &Path,
    worst: usize,
    weights_path: Option<&Path>,
    index_file: Option<&Path>,
) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let graph = RepoGraph::build(&index);
    let weights = match weights_path {
        Some(p) => {
            let toml = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", p.display()))?;
            repowise_health::HealthWeights::from_toml_str(&toml)?
        }
        None => repowise_health::HealthWeights::default(),
    };
    // One `GitAnalytics::collect` walk feeds both hot-path detection and
    // the organizational-signal markers below, rather than walking `git
    // log` twice for two different consumers of the same history.
    let analytics = repowise_git::GitAnalytics::collect(&root).ok();
    let hot_files = analytics
        .as_ref()
        .map(|a| hot_path_files(&index, a))
        .unwrap_or_default();
    // Coverage is optional: without it the two coverage markers simply
    // never fire, rather than every file scoring as untested.
    let coverage = repowise_core::coverage::CoverageData::load(&root).ok();
    // Org signals need one `git blame` per indexed file on top of the
    // history walk -- measured at several seconds on this port's own
    // workspace (see `repowise_git::org_signals`'s own module doc), the
    // same "not a cheap lookup, but this is the full-report command"
    // tradeoff `repowise refactor` already made for near-duplicate
    // detection (#304). Missing entirely (not a git repo, no history)
    // degrades to skipping these six markers, same as `coverage`.
    let org_signals = analytics
        .as_ref()
        .and_then(|a| repowise_git::org_signals::collect_org_signals(&root, &index, a).ok());
    let report = repowise_health::analyze_with_context(
        &index,
        &graph,
        &weights,
        &hot_files,
        coverage.as_ref(),
        org_signals.as_ref(),
    );

    println!("Repowise code health for {}", index.root.display());
    println!(
        "  average score: {:.1}/10 across {} file(s), {} marker(s) triggered",
        report.average_score,
        report.file_scores.len(),
        report.findings.len()
    );

    let by_kind = report.findings_by_kind();
    if !by_kind.is_empty() {
        println!("  markers by kind:");
        for (kind, count) in &by_kind {
            println!("    {:<20} {count}", kind.label());
        }
    }

    let worst_files: Vec<_> = report
        .file_scores
        .iter()
        .filter(|f| f.finding_count > 0)
        .take(worst)
        .collect();
    if !worst_files.is_empty() {
        println!("  lowest-scoring files:");
        for f in &worst_files {
            println!(
                "    {:<5.1} ({} marker(s))  {}",
                f.score,
                f.finding_count,
                display_path(&f.file, &index.root)
            );
        }
    }
    Ok(())
}

/// Parse the `--min-confidence` flag, mirroring the `get_dead_code` MCP
/// tool's accepted values so the two surfaces can't drift apart.
fn parse_min_confidence(raw: &str) -> anyhow::Result<DeadCodeConfidence> {
    match raw.to_ascii_lowercase().as_str() {
        "low" => Ok(DeadCodeConfidence::Low),
        "medium" => Ok(DeadCodeConfidence::Medium),
        "high" => Ok(DeadCodeConfidence::High),
        other => Err(anyhow::anyhow!(
            "min-confidence must be low/medium/high, got {other:?}"
        )),
    }
}

/// Render the dead-code listing. Split out from `cmd_dead_code` so the
/// filtering/truncation behaviour is testable without a real index on
/// disk -- the rest of the CLI's commands print inline, but this one has
/// enough logic between the analysis and the output to be worth pinning.
fn render_dead_code(
    candidates: Vec<DeadCodeCandidate>,
    root: &Path,
    threshold: DeadCodeConfidence,
    limit: usize,
) -> String {
    let matching: Vec<_> = candidates
        .into_iter()
        .filter(|c| c.confidence >= threshold)
        .collect();

    let mut out = String::new();
    out.push_str(&format!(
        "Repowise dead-code candidates for {}\n",
        root.display()
    ));

    if matching.is_empty() {
        out.push_str("  no candidates at or above the requested confidence tier\n");
        return out;
    }

    out.push_str(&format!(
        "  {} candidate(s) at or above `{}` confidence\n",
        matching.len(),
        threshold.label()
    ));

    for c in matching.iter().take(limit) {
        out.push_str(&format!(
            "    {:<7} {}:{}  {}\n",
            c.confidence.label(),
            display_path(&c.file, root),
            c.line,
            c.symbol
        ));
        for factor in &c.risk_factors {
            out.push_str(&format!("            - {factor}\n"));
        }
    }

    if matching.len() > limit {
        out.push_str(&format!(
            "  ... {} more not shown (raise --limit to see them)\n",
            matching.len() - limit
        ));
    }

    out.push_str("  Note: confidence describes this port's static call graph, not runtime\n");
    out.push_str("  safety. Reflection, dynamic dispatch, entry points, and #[test]\n");
    out.push_str("  functions are invisible to it -- review before deleting anything.\n");
    out
}

fn cmd_dead_code(
    path: &Path,
    min_confidence: &str,
    limit: usize,
    index_file: Option<&Path>,
) -> anyhow::Result<()> {
    let threshold = parse_min_confidence(min_confidence)?;
    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let graph = RepoGraph::build(&index);
    let candidates = repowise_health::find_dead_code(&index, &graph);

    print!(
        "{}",
        render_dead_code(candidates, &index.root, threshold, limit)
    );
    Ok(())
}

/// Index freshness, as reported by `repowise status`. Deliberately says
/// nothing about what the index *contains* -- that's `repowise
/// overview`'s job -- only about whether it still describes the tree on
/// disk.
#[derive(Debug, Default)]
struct StatusReport {
    /// `None` when no index exists yet: a valid state to report, not an
    /// error.
    indexed: Option<IndexedStatus>,
    /// Count of generated wiki pages under `.repowise/wiki`, if any.
    wiki_pages: usize,
}

#[derive(Debug)]
struct IndexedStatus {
    file_count: usize,
    /// Indexed files whose on-disk mtime is newer than the index itself.
    stale: Vec<PathBuf>,
    /// Indexed files that no longer exist on disk.
    missing: Vec<PathBuf>,
}

/// Compare each indexed file's mtime against the index's own mtime.
///
/// Deliberately filesystem-based rather than git-based: it works in a
/// repo with no git history, a shallow clone, or no git at all, and it
/// catches uncommitted edits that a `git diff` against the indexed
/// commit would miss. The tradeoff is that it can't see files that are
/// *new* since indexing -- finding those needs a full re-walk, which is
/// what `repowise update` does anyway.
fn collect_status(root: &Path) -> StatusReport {
    let mut report = StatusReport {
        wiki_pages: count_wiki_pages(root),
        ..Default::default()
    };

    let index_path = RepoIndex::index_path(root);

    // Prefer the slim sidecar (issue #333). This function needs only
    // each indexed file's path, and reading them out of the full index
    // means parsing every symbol and call edge first -- ~2s on this repo
    // in a release build, paid on every `repowise status` and on every
    // Claude Code session start. Falls back to the full index when the
    // sidecar is absent (an index built before it existed), stale, or
    // an unrecognized schema version.
    let files: Vec<PathBuf> = match RepoIndex::load_status(root) {
        Some(status) => status.files,
        None => {
            let Ok(index) = RepoIndex::load(root) else {
                return report;
            };
            index.files.iter().map(|f| f.path.clone()).collect()
        }
    };
    let Some(index_mtime) = mtime_of(&index_path) else {
        return report;
    };

    let mut stale = Vec::new();
    let mut missing = Vec::new();
    for path in &files {
        match mtime_of(path) {
            None => missing.push(path.clone()),
            Some(m) if m > index_mtime => stale.push(path.clone()),
            Some(_) => {}
        }
    }

    report.indexed = Some(IndexedStatus {
        file_count: files.len(),
        stale,
        missing,
    });
    report
}

fn mtime_of(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Count generated wiki pages.
///
/// Must recurse: `repowise docs` mirrors the repo's own tree under
/// `.repowise/wiki/`, so a repo whose sources live in subdirectories has
/// no pages at the wiki root at all. Shares `export`'s walk so the two
/// can't disagree about what counts as a page.
fn count_wiki_pages(root: &Path) -> usize {
    export::collect_pages(&repowise_docs::wiki_root(root)).len()
}

/// Render an index-freshness report. Pure, so every branch (no index,
/// clean, stale, missing files) is testable without touching disk.
fn render_status(report: &StatusReport, root: &Path, verbose: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!("Repowise status for {}\n", root.display()));

    match &report.indexed {
        None => {
            out.push_str("  index: none -- run `repowise init` to build one\n");
        }
        Some(idx) => {
            out.push_str(&format!("  index: {} file(s) indexed\n", idx.file_count));
            if idx.stale.is_empty() && idx.missing.is_empty() {
                out.push_str("  freshness: up to date\n");
            } else {
                out.push_str(&format!(
                    "  freshness: stale -- {} modified, {} missing since indexing\n",
                    idx.stale.len(),
                    idx.missing.len()
                ));
                if verbose {
                    for p in &idx.stale {
                        out.push_str(&format!("    modified  {}\n", display_path(p, root)));
                    }
                    for p in &idx.missing {
                        out.push_str(&format!("    missing   {}\n", display_path(p, root)));
                    }
                } else if idx.stale.len() + idx.missing.len() > 0 {
                    out.push_str("    (pass --verbose to list them)\n");
                }
                out.push_str("  run `repowise update` to re-index\n");
            }
            out.push_str(
                "  note: files created since indexing aren't detected here --\n\
                 \x20 finding those needs the full re-walk `repowise update` does.\n",
            );
        }
    }

    out.push_str(&format!(
        "  wiki: {}\n",
        match report.wiki_pages {
            0 => "no pages -- run `repowise docs`".to_string(),
            n => format!("{n} page(s) under .repowise/wiki"),
        }
    ));
    out
}

fn cmd_export(path: &Path, out: &Path, format: ExportFormat, force: bool) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    match format {
        ExportFormat::Markdown => {
            let wiki_root = repowise_docs::wiki_root(&root);
            let plan = export::plan(&wiki_root, out, force)?;
            let count = export::execute(&plan)?;
            println!(
                "exported {count} wiki page(s) from {} to {}",
                wiki_root.display(),
                out.display()
            );
        }
        ExportFormat::Index => {
            let index = RepoIndex::load(&root)?;
            // Record module paths for the languages that can't recompute
            // one without the repo on disk (issue #388). Computed here,
            // where the checkout exists and `repowise-graph` is already a
            // dependency -- `repowise-core` deliberately depends on no
            // other `repowise-*` crate, so it cannot derive these itself.
            let module_paths = disk_derived_module_paths(&index);
            let recorded = module_paths.len();
            let portable = repowise_core::portable::PortableIndex::from_index(&index)
                .with_module_paths(&index.root, module_paths)?;
            let dest = export::portable_index_dest(out, force)?;
            portable.save(&dest)?;
            println!(
                "exported {} file(s) as a portable index (schema v{}) to {}",
                portable.index.files.len(),
                portable.schema_version,
                dest.display()
            );
            match &portable.index.indexed_commit {
                Some(sha) => println!(
                    "  built at commit {sha} -- whoever reads this artifact is told when \
                     it has drifted from their checkout"
                ),
                // Not a warning about size or correctness but about
                // readability: without a commit to compare against,
                // every consumer's staleness answer is "unknown", which
                // is honest but useless.
                None => println!(
                    "  no indexed commit recorded, so readers cannot tell whether this is \
                     current -- re-run `repowise init` inside a git repository first"
                ),
            }
            if recorded > 0 {
                println!(
                    "  recorded {recorded} Rust/Go module path(s), so cross-repo resolution \
                     works for a workspace member backed by this artifact with no checkout."
                );
            }
            println!(
                "  safe to commit: paths are repo-relative and records are sorted, so the \
                 same commit exports identically on any machine."
            );
        }
        ExportFormat::JsonGraph => {
            let index = RepoIndex::load(&root)?;
            let graph = RepoGraph::build(&index);
            let doc = graph.to_json_graph(&index);
            let dest = export::json_graph_dest(out, force)?;
            // Pretty-printed rather than compact: an architecture model
            // is something people read and diff in review, and the size
            // difference doesn't matter for a file written once.
            std::fs::write(&dest, serde_json::to_string_pretty(&doc)?)?;
            println!(
                "exported {} node(s) and {} edge(s) to {}",
                doc.graph.nodes.len(),
                doc.graph.edges.len(),
                dest.display()
            );
            let unresolved = &doc.graph.metadata.unresolved;
            if unresolved.imports > 0 || unresolved.calls > 0 {
                // Say it out loud, not just in the file's metadata: the
                // graph is partial and a reader should know before they
                // draw conclusions from it.
                println!(
                    "  note: {} unresolved import(s) and {} unresolved call(s) have no",
                    unresolved.imports, unresolved.calls
                );
                println!("  edge (see graph.metadata.unresolved) -- absent edges do not");
                println!("  imply absent dependencies");
            }
        }
    }
    Ok(())
}

fn cmd_coverage(action: CoverageAction) -> anyhow::Result<()> {
    match action {
        CoverageAction::Add {
            reports,
            path,
            replace,
        } => cmd_coverage_add(&reports, &path, replace),
        CoverageAction::Status { path, worst } => cmd_coverage_status(&path, worst),
    }
}

fn cmd_coverage_add(reports: &[PathBuf], path: &Path, replace: bool) -> anyhow::Result<()> {
    use repowise_core::coverage::{self, CoverageData};

    let root = path.canonicalize()?;
    // Merge into existing coverage by default: the reference
    // auto-discovers and merges multiple sources, and a suite split
    // across CI shards produces one report per shard.
    let mut data = if replace {
        CoverageData::default()
    } else {
        CoverageData::load(&root).unwrap_or_default()
    };

    let mut total_unmatched = Vec::new();
    for report in reports {
        let text = std::fs::read_to_string(report)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", report.display()))?;
        let (parsed, summary) = coverage::ingest(&text, &root)
            .map_err(|e| anyhow::anyhow!("{}: {e}", report.display()))?;
        println!(
            "  {}: {} file(s), {} test context(s)",
            report.display(),
            summary.files_ingested,
            summary.tests_seen
        );
        total_unmatched.extend(summary.unmatched_paths);
        data.merge(parsed);
    }

    let saved = data.save(&root)?;
    println!(
        "ingested {} report(s) -> {} file(s) covered, {} test context(s) ({})",
        reports.len(),
        data.files.len(),
        data.per_test.len(),
        saved.display()
    );

    // Loud on purpose. Coverage whose paths don't line up with the repo
    // produces a clean-looking but empty result, which is the most
    // likely way this whole layer silently does nothing.
    if !total_unmatched.is_empty() {
        total_unmatched.sort();
        total_unmatched.dedup();
        println!(
            "  warning: {} path(s) in the report(s) matched no file under {}:",
            total_unmatched.len(),
            root.display()
        );
        for p in total_unmatched.iter().take(10) {
            println!("    {}", p.display());
        }
        if total_unmatched.len() > 10 {
            println!("    ... {} more", total_unmatched.len() - 10);
        }
    }
    Ok(())
}

fn cmd_coverage_status(path: &Path, worst: usize) -> anyhow::Result<()> {
    use repowise_core::coverage::CoverageData;

    let root = path.canonicalize()?;
    let Ok(data) = CoverageData::load(&root) else {
        println!("no coverage data -- run `repowise coverage add <REPORT>`");
        return Ok(());
    };

    let mut scored: Vec<(PathBuf, f64)> = data
        .files
        .keys()
        .filter_map(|p| data.line_coverage_of(p).map(|pct| (p.clone(), pct)))
        .collect();

    println!("Repowise coverage for {}", root.display());
    if scored.is_empty() {
        println!("  coverage recorded, but no file has any known lines");
        return Ok(());
    }

    let mean = scored.iter().map(|(_, p)| p).sum::<f64>() / scored.len() as f64;
    println!(
        "  {} file(s) measured, {:.1}% mean line coverage",
        scored.len(),
        mean
    );
    println!(
        "  per-test map: {}",
        if data.has_per_test_map() {
            format!("{} test context(s)", data.per_test.len())
        } else {
            "none -- reports carried no TN: records, so `impacted-tests` can't run".to_string()
        }
    );

    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    println!("  least-covered files:");
    for (file, pct) in scored.iter().take(worst) {
        println!("    {pct:>5.1}%  {}", display_path(file, &root));
    }
    Ok(())
}

fn cmd_impacted_tests(revspec: Option<&str>, path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let changed = repowise_git::changed_lines(&root, revspec)?;
    let coverage = repowise_core::coverage::CoverageData::load(&root).ok();
    let result = impacted::select(&changed, coverage.as_ref());
    print!(
        "{}",
        impacted::render(&result, revspec.unwrap_or("HEAD"), &root)
    );
    Ok(())
}

fn cmd_doctor(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let checks = doctor::run_checks(&root);
    print!("{}", doctor::render(&checks, &root));
    if doctor::any_failed(&checks) {
        // Warnings deliberately don't reach here: a degraded setup is
        // still a working one, and a nonzero exit would make `doctor`
        // useless in a CI gate that only cares about hard breakage.
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_hook(action: HookAction) -> anyhow::Result<()> {
    let message = match action {
        HookAction::Install { path } => hook::install(&path.canonicalize()?)?,
        HookAction::Uninstall { path } => hook::uninstall(&path.canonicalize()?)?,
        HookAction::Status { path } => hook::status(&path.canonicalize()?)?,
        HookAction::Rewrite { action } => return cmd_hook_rewrite(action),
    };
    println!("{message}");
    Ok(())
}

/// Runs one command through Distill's fail-open rewrite decision,
/// recording a skip for `saved --missed` accounting the same way `hook
/// rewrite apply` always has. Shared by that command and the Claude
/// Code `PreToolUse` adapter (`claude-hook pre-tool-use`), so the exact
/// same decision logic backs both surfaces rather than two copies of it
/// drifting apart.
fn distill_apply(input: &str) -> String {
    let decision = repowise_distill::decide(input);
    // Recording skips is what makes `saved --missed` real: it's the
    // feature auditing its own coverage, the difference between "the
    // hook is working" and "the hook is installed". Best-effort --
    // accounting must never break a command.
    if let repowise_distill::Decision::Skip(reason) = &decision {
        if let Ok(cwd) = std::env::current_dir() {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let dir = repowise_distill::store::store_dir(&cwd, home.as_deref());
            let program = input
                .split_whitespace()
                .next()
                .unwrap_or("")
                .rsplit('/')
                .next()
                .unwrap_or("");
            if !program.is_empty() {
                repowise_distill::ledger::record_skipped(&dir, program, reason.label());
            }
        }
    }
    match decision {
        repowise_distill::Decision::Rewrite { .. } => repowise_distill::rewrite::rewrite(input),
        // Every skip path returns the input verbatim. This is the
        // fail-open contract: the hook must be incapable of changing a
        // command it doesn't understand.
        repowise_distill::Decision::Skip(_) => input.trim().to_string(),
    }
}

/// `apply` reads stdin and writes stdout with no trailing newline --
/// it's called from a shell script that captures its output as a
/// command line, so a stray newline would end up inside the command.
fn cmd_hook_rewrite(action: RewriteAction) -> anyhow::Result<()> {
    use std::io::{Read, Write};

    let message = match action {
        RewriteAction::Install { path } => hook::rewrite_install(&path.canonicalize()?)?,
        RewriteAction::Uninstall { path } => hook::rewrite_uninstall(&path.canonicalize()?)?,
        RewriteAction::Status { path } => hook::rewrite_status(&path.canonicalize()?)?,
        RewriteAction::Apply => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            let out = distill_apply(&input);
            let mut stdout = std::io::stdout();
            write!(stdout, "{out}")?;
            stdout.flush()?;
            return Ok(());
        }
    };
    println!("{message}");
    Ok(())
}

/// `SessionStart`'s pure logic, split from `cmd_claude_hook` so it's
/// callable directly in tests without stdin/stdout plumbing: `None`
/// means "say nothing" (an unreadable root, or a directory this port
/// can't index at all -- SessionStart must never turn a non-repowise
/// project into a broken session).
///
/// Bootstraps the index via `repowise_parser::build_index` (the same
/// one implementation `init`/`update`/the background reindex job all
/// share) when none exists yet; otherwise reports freshness via the
/// same mtime-diffing `collect_status` already does for `repowise
/// status`, without auto-updating a stale one -- see
/// `ClaudeHookAction::SessionStart`'s own doc comment for why.
fn claude_hook_session_start(root: &Path) -> Option<serde_json::Value> {
    // `collect_status` already reports "no readable index" as
    // `indexed: None`, so ask it first and bootstrap only if it comes
    // back empty (issue #333).
    //
    // The previous shape called `RepoIndex::load` purely to test whether
    // an index existed, discarded the result, and then let
    // `collect_status` load it a second time -- two full 8.4 MB parses
    // to answer "is it stale?", which is why this hook measured ~3.9s in
    // a release build while `repowise status` measured ~2.0s. It runs on
    // every session start, with a person waiting.
    let additional_context = match collect_status(root).indexed {
        Some(indexed) => Some(if indexed.stale.is_empty() && indexed.missing.is_empty() {
            format!(
                "repowise: index is up to date ({} file(s)).",
                indexed.file_count
            )
        } else {
            format!(
                "repowise: index is stale ({} file(s) changed, {} removed since the \
                 last `repowise update`). MCP tool results may lag the working tree \
                 until you run `repowise update`.",
                indexed.stale.len(),
                indexed.missing.len(),
            )
        }),
        // No index, or one too damaged to read: build a fresh one, the
        // same behaviour as before.
        None => repowise_parser::build_index(root).ok().and_then(|index| {
            let file_count = index.files.len();
            save_index_with_sidecars(&index).ok().map(|_| {
                format!(
                    "repowise: indexed {file_count} file(s) for the first time. MCP tools \
                     (search_codebase, get_context, get_risk, ...) are now available."
                )
            })
        }),
    };

    additional_context.map(|additional_context| {
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": additional_context,
            }
        })
    })
}

/// `PreToolUse`'s pure logic, split out for the same reason
/// `claude_hook_session_start` is. `None` on any shape this hook
/// doesn't recognize (unparseable JSON, a tool other than `Bash`, a
/// command Distill leaves unchanged) -- fail-open in the same sense
/// `distill_apply` already is: nothing to say means nothing is printed,
/// never an error.
fn claude_hook_pre_tool_use(input: &str) -> Option<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(input).ok()?;
    if parsed.get("tool_name").and_then(|v| v.as_str()) != Some("Bash") {
        return None;
    }
    let command = parsed
        .pointer("/tool_input/command")
        .and_then(|v| v.as_str())?;

    let rewritten = distill_apply(command);
    if rewritten == command {
        // Nothing to say: emitting an "unchanged" updatedInput would be
        // noise, and there's no decision to report on a command this
        // hook isn't touching.
        return None;
    }

    Some(serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": { "command": rewritten },
        }
    }))
}

/// How many dependents to name before falling back to a bare count.
///
/// This text is injected into the model's context after *every* matching
/// edit, so it is a per-edit token tax. Naming the whole list of a
/// heavily-imported file (70 of them, on this repo's own
/// `repowise-core/src/lib.rs`) would cost more than it informs -- the
/// actionable part is the magnitude, plus enough names to start looking.
const POST_TOOL_USE_NAMED_DEPENDENTS: usize = 5;

/// `PostToolUse`: after an edit, say what else imports the file that was
/// just changed (issue #333).
///
/// Matched to `Edit`/`Write` only, deliberately. `Read`, `Grep` and
/// `Glob` are far more frequent and the answer is not actionable at that
/// moment -- the blast radius of a file matters when you have just
/// changed it. Every match costs context tokens, so the narrow matcher
/// is the point rather than an omission.
///
/// Reads the `.repowise/dependents.json` sidecar, never the index: this
/// runs after every matching tool call, and `RepoIndex::load` is ~2s in
/// a release build on this repo. A missing, stale, or unreadable sidecar
/// means this says nothing at all, which is the right failure -- silence
/// reads as "no information", a stale blast radius reads as fact.
fn claude_hook_post_tool_use(root: &Path, input: &str) -> Option<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(input).ok()?;
    let tool = parsed.get("tool_name").and_then(|v| v.as_str())?;
    if !matches!(tool, "Edit" | "Write") {
        return None;
    }
    let file = parsed
        .pointer("/tool_input/file_path")
        .and_then(|v| v.as_str())?;

    // The sidecar is keyed by repo-relative path; the hook receives an
    // absolute one.
    let path = Path::new(file);
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string();

    let dependents = repowise_graph::load_dependents(root)?;
    let who = dependents.of(&relative);
    if who.is_empty() {
        // Nothing imports it, or it isn't indexed. Either way there is
        // no blast radius worth a line of context.
        return None;
    }

    let named = who
        .iter()
        .take(POST_TOOL_USE_NAMED_DEPENDENTS)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let context = if who.len() > POST_TOOL_USE_NAMED_DEPENDENTS {
        format!(
            "repowise: {} file(s) import {relative}, including {named}. \
             Use `get_context` for the full list before assuming this change is local.",
            who.len()
        )
    } else {
        format!(
            "repowise: {} file(s) import {relative}: {named}.",
            who.len()
        )
    };

    Some(serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": context,
        }
    }))
}

/// See `ClaudeHookAction`'s own doc comments for what each event does.
/// Both are fail-open by construction (see
/// `claude_hook_session_start`/`claude_hook_pre_tool_use`): a hook that
/// can break session start or block a tool call would be worse than one
/// that occasionally says nothing.
fn cmd_claude_hook(action: ClaudeHookAction) -> anyhow::Result<()> {
    use std::io::Read;

    // Claude Code always sends a JSON payload on stdin for both events;
    // `SessionStart` doesn't need any of its fields (it reads the
    // *current* index/tree state directly), but draining it either way
    // avoids ever leaving a parent process's pipe write blocked on an
    // unread payload.
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);

    let output = match action {
        ClaudeHookAction::SessionStart => {
            let Ok(root) = std::env::current_dir() else {
                return Ok(());
            };
            claude_hook_session_start(&root)
        }
        ClaudeHookAction::PreToolUse => claude_hook_pre_tool_use(&input),
        ClaudeHookAction::PostToolUse => match std::env::current_dir() {
            Ok(root) => claude_hook_post_tool_use(&root, &input),
            Err(_) => None,
        },
    };

    if let Some(output) = output {
        println!("{output}");
    }
    Ok(())
}

fn cmd_status(path: &Path, verbose: bool) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let report = collect_status(&root);
    print!("{}", render_status(&report, &root, verbose));
    Ok(())
}

/// Bucket a 0-10 change-risk score into a word. Purely presentational --
/// the underlying score is a documented heuristic, not a calibrated
/// probability, so the bands are round numbers rather than thresholds
/// derived from any corpus.
fn risk_band(score: f64) -> &'static str {
    match score {
        s if s >= 7.0 => "high",
        s if s >= 4.0 => "moderate",
        _ => "low",
    }
}

/// Render a change-risk assessment. Split out from `cmd_risk` so the
/// formatting and banding are testable without a git repo present.
fn render_risk(risk: &repowise_git::ChangeRisk) -> String {
    let mut out = String::new();
    out.push_str(&format!("Repowise change risk for {}\n", risk.revspec));
    out.push_str(&format!(
        "  score: {:.1}/10 ({})\n",
        risk.score,
        risk_band(risk.score)
    ));
    out.push_str(&format!(
        "  diff shape: {} file(s), +{} / -{} line(s), {} subsystem(s)\n",
        risk.files_touched, risk.lines_added, risk.lines_deleted, risk.subsystems_touched
    ));
    out.push_str(&format!(
        "  concentration: {:.2} (0.00 = all in one file, 1.00 = spread evenly)\n",
        risk.concentration
    ));
    out.push_str(&format!(
        "  author: {} ({} prior commit(s) in this repo)\n",
        risk.author, risk.author_prior_commits
    ));
    out.push_str(
        "  Note: a fixed-weight diff-shape heuristic, not a calibrated\n\
         \x20 probability -- see repowise-git's change_risk docs for the formula.\n",
    );
    out
}

fn cmd_risk(revspec: Option<&str>, path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let risk = repowise_git::change_risk(&root, revspec)?;
    print!("{}", render_risk(&risk));
    Ok(())
}

fn cmd_hotspots(path: &Path, top: usize, index_file: Option<&Path>) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let analytics = repowise_git::GitAnalytics::collect(&root)?;
    let hotspots = repowise_git::hotspots(&index, &analytics);

    println!(
        "Repowise hotspots for {} ({} commit(s) analyzed)",
        index.root.display(),
        analytics.commit_count
    );
    if hotspots.is_empty() {
        println!("  No indexed file has git history under this root.");
        return Ok(());
    }
    println!(
        "  {:<10} {:<8} {:<6} {:<11} {:<8} {:<10} file (last touched by)",
        "score", "raw score", "churn", "complexity", "bugfixes", "last"
    );
    for h in hotspots.iter().take(top) {
        let last = h
            .last_touch
            .as_ref()
            .map(|(hash, author)| format!("{hash} {author}"))
            .unwrap_or_default();
        println!(
            "  {:<10.1} {:<8} {:<6} {:<11} {:<8} {:<10} {}",
            h.decayed_score,
            h.score,
            h.churn,
            h.total_complexity,
            h.bugfix_commits,
            last,
            display_path(&h.file, &index.root)
        );
    }
    Ok(())
}

fn cmd_ownership(file: &Path, path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let target = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let target = target.canonicalize().unwrap_or(target);

    let ownership = repowise_git::ownership_of(&root, &target)?;
    println!("{}", display_path(&target, &root));
    for o in &ownership {
        println!(
            "  {:>5.1}%  ({} line(s))  {}",
            o.percentage, o.lines, o.author
        );
    }
    println!(
        "{}",
        render_bus_factor(repowise_git::bus_factor(&ownership))
    );
    Ok(())
}

/// Phrase a bus factor so the number can't be read as a quality score.
/// A bare "bus factor: 1" invites the reading "one owner, tidy" -- the
/// opposite of what it means.
fn render_bus_factor(bus_factor: usize) -> String {
    match bus_factor {
        0 => "  bus factor: n/a (no blameable lines)".to_string(),
        1 => "  bus factor: 1 -- one author wrote most of this file".to_string(),
        n => format!("  bus factor: {n} -- {n} authors between them wrote most of this file"),
    }
}

fn cmd_coupled(file: &Path, path: &Path, top: usize) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let target = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let target = target.canonicalize().unwrap_or(target);

    let analytics = repowise_git::GitAnalytics::collect(&root)?;
    let coupled = analytics.coupled_files(&target, top);

    println!("{}", display_path(&target, &root));
    if coupled.is_empty() {
        println!("  No co-change coupling found (or too little history).");
        return Ok(());
    }
    println!("  Most often changed alongside:");
    for (f, count) in &coupled {
        println!("    {:<4} {}", count, display_path(f, &root));
    }
    Ok(())
}

fn cmd_coupling(path: &Path, top: usize) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let analytics = repowise_git::GitAnalytics::collect(&root)?;
    let pairs = analytics.top_co_changed_pairs(top);

    if pairs.is_empty() {
        println!("No co-change coupling found (or too little history).");
        return Ok(());
    }
    println!("Most-coupled file pairs in {}:", root.display());
    for (a, b, count) in &pairs {
        println!(
            "  {:<4} {} <-> {}",
            count,
            display_path(a, &root),
            display_path(b, &root)
        );
    }
    Ok(())
}

fn cmd_commits(path: &Path, limit: usize) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let commits = repowise_git::collect_recent_commits(&root, limit)?;

    if commits.is_empty() {
        println!("No commits found under {}", root.display());
        return Ok(());
    }
    println!(
        "{} most recent commit(s) under {}:",
        commits.len(),
        root.display()
    );
    for c in &commits {
        let date = chrono::DateTime::from_timestamp(c.timestamp, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "unknown date".to_string());
        let short_hash: String = c.hash.chars().take(7).collect();
        println!(
            "  {short_hash}  {date}  {:<20}  {} file(s) touched  {}",
            c.author,
            c.files.len(),
            c.message
        );
    }
    Ok(())
}

fn cmd_docs(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;
    let graph = RepoGraph::build(&index);
    let health = repowise_health::analyze(&index, &graph);
    let summary = repowise_docs::generate(&index, &graph, &health)?;

    let (new, changed, unchanged) = summary.counts();
    println!(
        "Generated {} wiki page(s) under {}/.repowise/wiki",
        summary.pages.len(),
        index.root.display()
    );
    println!("  {new} new, {changed} changed, {unchanged} unchanged (by source content hash)");
    Ok(())
}

fn cmd_doc_coverage(path: &Path, verbose: bool, index_file: Option<&Path>) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let report = repowise_docs::check_freshness(&index);
    let (missing, fresh, stale) = report.counts();

    println!("Doc coverage for {}", index.root.display());
    println!("  {fresh} fresh, {stale} stale, {missing} missing");
    if verbose {
        for entry in &report.entries {
            let rel = entry.file.strip_prefix(&index.root).unwrap_or(&entry.file);
            let status = match entry.status {
                repowise_docs::FreshnessStatus::Missing => "missing",
                repowise_docs::FreshnessStatus::Fresh => "fresh",
                repowise_docs::FreshnessStatus::Stale => "stale",
            };
            println!("  {status:<8} {}", rel.display());
        }
    }
    Ok(())
}

fn cmd_decisions(
    path: &Path,
    for_file: Option<&Path>,
    index_file: Option<&Path>,
) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let (mut decisions, inferred_state) = repowise_adr::mine_reporting(&index)?;

    if let Some(f) = for_file {
        let target = if f.is_absolute() {
            f.to_path_buf()
        } else {
            root.join(f)
        };
        let target = target.canonicalize().unwrap_or(target);
        decisions.retain(|d| d.linked_files.contains(&target));
    }

    println!(
        "Repowise decisions for {} ({} found)",
        index.root.display(),
        decisions.len()
    );
    // Printed whether or not anything was found: "no inferred decisions"
    // and "the pass that infers them never ran" are different facts, and
    // an empty list looks the same either way.
    println!("  {}", inferred_state.describe());
    if decisions.is_empty() {
        println!(
            "  No decisions found (docs/adr/*.md, decision-like commit messages, merged PR \
             bodies, code comments, inline markers, CHANGELOG sections, and README/ \
             ARCHITECTURE prose)."
        );
        return Ok(());
    }

    for d in &decisions {
        let source_label = match &d.source {
            repowise_adr::DecisionSource::Adr { file } => {
                format!("ADR ({})", display_path(file, &index.root))
            }
            repowise_adr::DecisionSource::CommitMessage { hash, author } => {
                format!("commit {} by {author}", &hash[..hash.len().min(7)])
            }
            repowise_adr::DecisionSource::PullRequest { number, author } => {
                format!("PR #{number} by {author}")
            }
            repowise_adr::DecisionSource::CodeComment { file, line } => {
                format!("comment ({}:{line})", display_path(file, &index.root))
            }
            repowise_adr::DecisionSource::InlineMarker { file, line, marker } => {
                format!(
                    "{marker} marker ({}:{line})",
                    display_path(file, &index.root)
                )
            }
            repowise_adr::DecisionSource::Changelog { file, section } => {
                format!("{section} (changelog: {})", display_path(file, &index.root))
            }
            repowise_adr::DecisionSource::ReadmeMining {
                file,
                line,
                heading,
            } => {
                format!("\"{heading}\" ({}:{line})", display_path(file, &index.root))
            }
            repowise_adr::DecisionSource::Inferred { file, line, model } => {
                format!(
                    "LLM-INFERRED from {}:{line} by model {model} -- not a written decision",
                    display_path(file, &index.root)
                )
            }
            repowise_adr::DecisionSource::Manual { recorded_at } => {
                format!("recorded via `repowise decide` on {recorded_at}")
            }
        };
        let status = d.status.as_deref().unwrap_or("-");
        // The marker goes in the id column, where it can't be scrolled
        // past: a reader scanning titles must not have to reach the
        // source line to learn this one was guessed.
        let marker = if d.source.is_inferred() { "~" } else { " " };
        println!("{marker} {:<10} {:<10} {}", d.id, status, d.title);
        println!(
            "    source: {source_label} (confidence: {:.2})",
            d.confidence
        );
        if let Some(target) = &d.superseded_by {
            println!("    superseded by: {target}");
        }
        if !d.linked_files.is_empty() {
            println!("    linked files:");
            for f in &d.linked_files {
                println!("      {}", display_path(f, &index.root));
            }
        }
    }
    Ok(())
}

fn cmd_decide(path: &Path, title: String, rationale: String) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    if title.trim().is_empty() {
        anyhow::bail!("title must not be empty");
    }
    if rationale.trim().is_empty() {
        anyhow::bail!("rationale must not be empty");
    }

    let recorded_at = chrono::Utc::now().to_rfc3339();
    let mut store = repowise_adr::ManualDecisionStore::load(&root);
    let decision = store.record(&root, title, rationale, recorded_at)?;

    println!("Recorded {} -- {}", decision.id, decision.title);
    println!("  Run `repowise decisions` to see it alongside every other mined decision.");
    Ok(())
}

fn cmd_docker_stages(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let (stages, edges) = repowise_parser::collect_docker_stages(&root)?;

    if stages.is_empty() {
        println!("No Dockerfiles found under {}", root.display());
        return Ok(());
    }

    println!(
        "{} Docker build stage(s) found under {}",
        stages.len(),
        root.display()
    );

    let mut by_file: std::collections::BTreeMap<&Path, Vec<&repowise_core::docker::DockerStage>> =
        std::collections::BTreeMap::new();
    for stage in &stages {
        by_file.entry(&stage.file).or_default().push(stage);
    }

    for (file, file_stages) in by_file {
        println!("  {}", display_path(file, &root));
        for stage in &file_stages {
            let label = stage
                .name
                .as_deref()
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("stage {}", stage.index));
            println!(
                "    [{}] {label} ({})  lines {}-{}",
                stage.index, stage.base_image, stage.start_line, stage.end_line
            );
            for edge in edges
                .iter()
                .filter(|e| e.file == stage.file && e.from_stage == stage.index)
            {
                let target = &file_stages
                    .iter()
                    .find(|s| s.index == edge.to_stage)
                    .expect("edge target stage must exist in the same file");
                let target_label = target
                    .name
                    .as_deref()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| format!("stage {}", target.index));
                println!("      copies from: {target_label} (line {})", edge.line);
            }
        }
    }
    Ok(())
}

fn cmd_sql(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let (objects, edges) = repowise_sql::collect_sql(&root)?;

    if objects.is_empty() {
        println!("No SQL objects found under {}", root.display());
        return Ok(());
    }

    println!(
        "{} SQL object(s), {} lineage edge(s) found under {}",
        objects.len(),
        edges.len(),
        root.display()
    );

    let mut by_file: std::collections::BTreeMap<&Path, Vec<&repowise_core::sql::SqlObject>> =
        std::collections::BTreeMap::new();
    for object in &objects {
        by_file.entry(&object.file).or_default().push(object);
    }

    for (file, file_objects) in by_file {
        println!("  {}", display_path(file, &root));
        for object in &file_objects {
            let columns = if object.columns.is_empty() {
                String::new()
            } else {
                format!("  columns: {}", object.columns.join(", "))
            };
            println!(
                "    [{}] {}  lines {}-{}{columns}",
                object.kind.label(),
                object.name,
                object.start_line,
                object.end_line
            );
        }
        for edge in edges.iter().filter(|e| e.from == *file) {
            let target = match &edge.resolved_file {
                Some(f) => display_path(f, &root),
                None => "unresolved".to_string(),
            };
            let keyword = match edge.kind {
                repowise_core::sql::LineageKind::Ref => "ref",
                repowise_core::sql::LineageKind::Source => "source",
            };
            println!(
                "      {keyword}: {} -> {target} (line {})",
                edge.name, edge.line
            );
        }
    }
    Ok(())
}

fn cmd_openapi(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let objects = repowise_openapi::collect_openapi(&root)?;

    if objects.is_empty() {
        println!("No OpenAPI documents found under {}", root.display());
        return Ok(());
    }

    println!(
        "{} OpenAPI object(s) found under {}",
        objects.len(),
        root.display()
    );

    let mut by_file: std::collections::BTreeMap<
        &Path,
        Vec<&repowise_core::openapi::OpenApiObject>,
    > = std::collections::BTreeMap::new();
    for object in &objects {
        by_file.entry(&object.file).or_default().push(object);
    }

    for (file, file_objects) in by_file {
        println!("  {}", display_path(file, &root));
        for object in &file_objects {
            let fields = if object.fields.is_empty() {
                String::new()
            } else {
                format!("  fields: {}", object.fields.join(", "))
            };
            println!(
                "    [{}] {}  line {}{fields}",
                object.kind.label(),
                object.name,
                object.start_line
            );
        }
    }
    Ok(())
}

fn cmd_protobuf(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let objects = repowise_protobuf::collect_protobuf(&root)?;

    if objects.is_empty() {
        println!("No protobuf objects found under {}", root.display());
        return Ok(());
    }

    println!(
        "{} protobuf object(s) found under {}",
        objects.len(),
        root.display()
    );

    let mut by_file: std::collections::BTreeMap<&Path, Vec<&repowise_core::protobuf::ProtoObject>> =
        std::collections::BTreeMap::new();
    for object in &objects {
        by_file.entry(&object.file).or_default().push(object);
    }

    for (file, file_objects) in by_file {
        println!("  {}", display_path(file, &root));
        for object in &file_objects {
            let fields = if object.fields.is_empty() {
                String::new()
            } else {
                format!("  fields: {}", object.fields.join(", "))
            };
            println!(
                "    [{}] {}  line {}{fields}",
                object.kind.label(),
                object.name,
                object.start_line
            );
        }
    }
    Ok(())
}

fn cmd_graphql(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let objects = repowise_graphql::collect_graphql(&root)?;

    if objects.is_empty() {
        println!("No GraphQL SDL documents found under {}", root.display());
        return Ok(());
    }

    println!(
        "{} GraphQL object(s) found under {}",
        objects.len(),
        root.display()
    );

    let mut by_file: std::collections::BTreeMap<
        &Path,
        Vec<&repowise_core::graphql::GraphQlObject>,
    > = std::collections::BTreeMap::new();
    for object in &objects {
        by_file.entry(&object.file).or_default().push(object);
    }

    for (file, file_objects) in by_file {
        println!("  {}", display_path(file, &root));
        for object in &file_objects {
            let fields = if object.fields.is_empty() {
                String::new()
            } else {
                format!("  fields: {}", object.fields.join(", "))
            };
            println!(
                "    [{}] {}  line {}{fields}",
                object.kind.label(),
                object.name,
                object.start_line
            );
        }
    }
    Ok(())
}

fn cmd_terraform(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let (resources, modules) = repowise_terraform::collect_terraform(&root)?;

    if resources.is_empty() && modules.is_empty() {
        println!(
            "No Terraform resource/module blocks found under {}",
            root.display()
        );
        return Ok(());
    }

    println!(
        "{} resource(s), {} module(s) found under {}",
        resources.len(),
        modules.len(),
        root.display()
    );

    let mut files: std::collections::BTreeSet<&Path> = std::collections::BTreeSet::new();
    files.extend(resources.iter().map(|r| r.file.as_path()));
    files.extend(modules.iter().map(|m| m.file.as_path()));

    for file in files {
        println!("  {}", display_path(file, &root));
        for r in resources.iter().filter(|r| r.file == file) {
            println!(
                "    [resource] {}.{}  line {}",
                r.resource_type, r.name, r.start_line
            );
        }
        for m in modules.iter().filter(|m| m.file == file) {
            let source = match &m.source {
                Some(s) => format!("  source: {s}"),
                None => String::new(),
            };
            println!("    [module] {}  line {}{source}", m.name, m.start_line);
        }
    }
    Ok(())
}

fn cmd_external_deps(path: &Path) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let deps = repowise_external_deps::collect_dependencies(&root)?;

    if deps.is_empty() {
        println!("No third-party dependencies found under {}", root.display());
        return Ok(());
    }

    println!(
        "{} third-party dependenc{} found under {}",
        deps.len(),
        if deps.len() == 1 { "y" } else { "ies" },
        root.display()
    );

    let files: std::collections::BTreeSet<&Path> = deps.iter().map(|d| d.file.as_path()).collect();

    for file in files {
        println!("  {}", display_path(file, &root));
        for d in deps.iter().filter(|d| d.file == file) {
            let version = d.version.as_deref().unwrap_or("(unversioned)");
            println!(
                "    [{}/{}] {} {version}",
                d.ecosystem,
                d.kind.label(),
                d.name
            );
        }
    }
    Ok(())
}

fn cmd_refactor(
    path: &Path,
    kind: Option<&str>,
    limit: usize,
    index_file: Option<&Path>,
) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let graph = RepoGraph::build(&index);

    let kind_filter = kind
        .map(|k| match k {
            "break-import-cycle" | "split-god-class" | "split-by-cohesion"
            | "extract-duplicate" => Ok(k.to_string()),
            other => anyhow::bail!(
                "unknown kind {other:?} -- expected break-import-cycle, split-god-class, \
                 split-by-cohesion, or extract-duplicate"
            ),
        })
        .transpose()?;

    let mut candidates = repowise_refactor::find_refactor_candidates(&index, &graph);
    if let Some(k) = &kind_filter {
        candidates.retain(|c| c.kind.label() == k);
    }
    let total = candidates.len();

    println!(
        "Repowise refactor candidates for {} ({total} found)",
        index.root.display(),
    );
    if candidates.is_empty() {
        println!(
            "  No structural refactor candidates found (import cycles, god classes, \
             low-cohesion classes, duplicate/near-duplicate functions)."
        );
        return Ok(());
    }
    println!(
        "  Read-only: these name a problem and where it is. Nothing here generates a diff \
         or writes to a file -- this port doesn't do that (see issue #304)."
    );

    let shown = if limit == 0 { total } else { limit.min(total) };
    if shown < total {
        println!(
            "  Showing the {shown} strongest of {total} (ranked exact-duplicate first, then \
             by descending overlap within extract-duplicate; --limit 0 for all, --kind to \
             narrow)."
        );
    }

    for c in candidates.iter().take(shown) {
        println!("  {:<20} {}", c.kind.label(), c.title);
        println!("    {}", c.rationale);
        if !c.files.is_empty() {
            println!("    files: {}", c.files.join(", "));
        }
        if !c.symbols.is_empty() {
            println!("    symbols: {}", c.symbols.join(", "));
        }
    }
    Ok(())
}

fn cmd_tour(
    path: &Path,
    from: Option<&Path>,
    max_steps: usize,
    format: &str,
    index_file: Option<&Path>,
) -> anyhow::Result<()> {
    let as_markdown = match format {
        "text" => false,
        "markdown" => true,
        other => anyhow::bail!("--format must be text or markdown, got {other:?}"),
    };

    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;
    let graph = RepoGraph::build(&index);
    let health = repowise_health::analyze(&index, &graph);

    // Hotspot data is a ranking tie-break, not a requirement: an
    // un-versioned checkout still gets a tour, the same way `get_risk`
    // and `/api/hotspots` degrade rather than erroring.
    let hotspots: HashMap<PathBuf, usize> = match repowise_git::GitAnalytics::collect(&root) {
        Ok(analytics) => repowise_git::hotspots(&index, &analytics)
            .into_iter()
            .map(|h| (h.file, h.score))
            .collect(),
        Err(_) => HashMap::new(),
    };

    let opts = repowise_tour::TourOptions {
        max_steps,
        from: from.map(|p| p.to_path_buf()),
    };
    let tour = repowise_tour::build_tour(&index, &graph, Some(&health), &hotspots, &opts)?;

    if tour.steps.is_empty() {
        println!(
            "No tour: nothing in this index has extracted symbols to read. \
             Structural-tier languages and empty files are indexed but make no tour stops."
        );
        return Ok(());
    }

    let scope = match &tour.rooted_at {
        Some(f) => format!(" rooted at {}", display_path(f, &index.root)),
        None => String::new(),
    };
    let shown = tour.steps.len();
    let heading = format!(
        "Repowise tour of {}{scope} -- {shown} of {} file(s) considered",
        index.root.display(),
        tour.considered
    );

    if as_markdown {
        println!("# {heading}\n");
        println!(
            "Read in this order: nothing is introduced before what it is built out of. \
             Foundations first, entry points last.\n"
        );
    } else {
        println!("{heading}");
        println!(
            "  Read in order -- foundations first, entry points last. \
             Nothing appears before what it is built out of."
        );
        if shown < tour.considered {
            println!(
                "  Showing the {shown} most depended-on; --max-steps 0 for every file, \
                 --from <FILE> to scope to one file's dependency closure."
            );
        }
    }

    for step in &tour.steps {
        let file = display_path(&step.file, &index.root);
        if as_markdown {
            println!("## {}. {file}\n", step.position);
            println!("{}\n", step.why());
            println!(
                "- role: `{}` | {} symbol(s), {} line(s)",
                step.role.label(),
                step.symbols,
                step.lines
            );
            println!(
                "- imported by {} file(s), imports {} file(s)",
                step.dependents, step.dependencies
            );
            match step.health {
                Some(h) => println!("- health: {h:.1}/10"),
                None => println!("- health: not measured"),
            }
            println!();
        } else {
            println!("  {:>2}. {:<14} {file}", step.position, step.role.label());
            println!("      {}", step.why());
        }
    }
    Ok(())
}

fn cmd_security(
    path: &Path,
    min_severity: Option<&str>,
    index_file: Option<&Path>,
) -> anyhow::Result<()> {
    let root = path.canonicalize()?;
    let index = load_index(&root, index_file)?;

    let min_rank = min_severity
        .map(|s| match s {
            "high" => Ok(repowise_security::Severity::High),
            "medium" => Ok(repowise_security::Severity::Medium),
            "low" => Ok(repowise_security::Severity::Low),
            other => anyhow::bail!("--min-severity must be high, medium, or low, got {other:?}"),
        })
        .transpose()?;

    let mut findings = repowise_security::scan(&index);
    if let Some(min_rank) = min_rank {
        findings.retain(|f| f.severity >= min_rank);
    }

    println!(
        "Repowise security findings for {} ({} found)",
        index.root.display(),
        findings.len(),
    );
    if findings.is_empty() {
        println!(
            "  No signature-based findings (hardcoded AWS/GitHub/Slack credentials, PEM \
             private-key blocks, credential-shaped literal assignments)."
        );
        return Ok(());
    }
    println!(
        "  Signature-based only -- no dependency-CVE checking, no injection-shape detection. \
         See repowise-security's own module doc for why."
    );

    for f in &findings {
        println!(
            "  {:<6} {:<24} {}:{} -- {}",
            f.severity.label(),
            f.kind.label(),
            f.file.display(),
            f.line,
            f.message,
        );
    }
    Ok(())
}

/// The workspace TOML to run with: the explicit flag, else
/// `REPOWISE_WORKSPACE` (issue #333).
///
/// The Claude Code plugin starts the MCP server from a static
/// `.mcp.json`, whose `args` cannot be conditional -- so without this,
/// every `repo` parameter added in #337 answered `requires a workspace;
/// start the MCP server with --workspace` and the plugin could reach
/// none of it. An env var is how this repo already gates its other
/// optional features (`REPOWISE_LLM_BASE_URL`, `REPOWISE_WEBHOOK_SECRET`),
/// and it needs no change to the plugin manifest at all.
///
/// Deliberately *not* discovery of a conventional filename: the
/// workspace file has no established name here, and inventing one would
/// make `serve` change behaviour based on a file appearing next to it.
///
/// An unusable value is a hard error rather than a quiet fall back to
/// single-repo mode -- a typo in a globally-exported variable would
/// otherwise leave every `repo="all"` call reporting "no workspace
/// configured" with nothing pointing at the cause.
fn resolve_workspace(flag: Option<PathBuf>) -> anyhow::Result<Option<PathBuf>> {
    if let Some(path) = flag {
        return Ok(Some(path));
    }
    match std::env::var("REPOWISE_WORKSPACE") {
        Ok(v) if !v.trim().is_empty() => {
            let path = PathBuf::from(v.trim());
            if !path.exists() {
                anyhow::bail!(
                    "REPOWISE_WORKSPACE is set to {}, which does not exist -- \
                     unset it or point it at a workspace TOML file",
                    path.display()
                );
            }
            Ok(Some(path))
        }
        _ => Ok(None),
    }
}

fn cmd_serve(path: &Path, workspace: Option<PathBuf>) -> anyhow::Result<()> {
    let workspace = resolve_workspace(workspace)?;
    let root = path.canonicalize()?;
    // The rest of the CLI is synchronous; only the MCP server needs an
    // async runtime, so build one here rather than making `main` async.
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(repowise_mcp::run(root, workspace))
}

fn cmd_serve_dashboard(
    path: &Path,
    addr: std::net::SocketAddr,
    static_dir: Option<PathBuf>,
    workspace: Option<PathBuf>,
    poll: Option<u64>,
) -> anyhow::Result<()> {
    let workspace = resolve_workspace(workspace)?;
    let root = path.canonicalize()?;
    let static_dir = static_dir.or_else(|| {
        let exe = std::env::current_exe().ok();
        let candidates = [
            PathBuf::from("web/dist"),
            PathBuf::from("C:\\dev\\rusty_repo_wise\\web\\dist"),
            exe.as_ref()
                .and_then(|p| p.parent().map(|d| d.join("web/dist")))
                .unwrap_or_default(),
            exe.as_ref()
                .and_then(|p| {
                    p.parent()
                        .and_then(|d| d.parent())
                        .and_then(|d| d.parent())
                        .map(|d| d.join("web/dist"))
                })
                .unwrap_or_default(),
            PathBuf::from("crates/repowise-web/dist"),
        ];
        candidates.into_iter().find(|p| p.exists() && p.is_dir())
    });

    if static_dir.is_none() {
        println!(
            "No --static-dir given and web/dist not found: serving the JSON API only (no frontend). \
             Build one with `cd web && npm run build` and pass \
             --static-dir web/dist."
        );
    } else if let Some(ref dir) = static_dir {
        println!("Serving static frontend from {}", dir.display());
    }
    println!("Dashboard server listening on http://{addr}");
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(repowise_server::serve(
        root, addr, static_dir, workspace, poll,
    ))
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_repos(workspace: &Path) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    println!(
        "{} repo(s) configured in {}:",
        repos.len(),
        workspace.display()
    );
    for repo in &repos {
        let status = repowise_workspace::repo_status(repo);
        match status.file_count {
            Some(file_count) => println!(
                "  {} — {} ({file_count} file(s) indexed, {} index{})",
                status.name,
                status.path.display(),
                status.source.map(|s| s.label()).unwrap_or("unknown-source"),
                staleness_suffix(status.stale),
            ),
            None => println!(
                "  {} — {} (not indexed; run `repowise init` there)",
                status.name,
                status.path.display()
            ),
        }
    }
    Ok(())
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_co_changes(workspace: &Path, top: usize) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    for report in repowise_workspace::workspace_co_changes(&repos, top) {
        println!("{} — {}", report.name, report.path.display());
        if !report.available {
            println!("  No git history found (or not a git repo).");
            continue;
        }
        if report.pairs.is_empty() {
            println!("  No co-change coupling found (or too little history).");
            continue;
        }
        for pair in &report.pairs {
            println!(
                "  {:<4} {} <-> {}",
                pair.count,
                display_path(&pair.file_a, &report.path),
                display_path(&pair.file_b, &report.path)
            );
        }
    }
    Ok(())
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_architecture(workspace: &Path) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    warn_resolution_blind_spots(&repos);
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let report = repowise_workspace::workspace_architecture(&repos);

    for status in &report.repos {
        let indexed = if status.indexed {
            format!("{} file(s) indexed", status.file_count.unwrap_or(0))
        } else {
            "not indexed".to_string()
        };
        println!("{} — {} ({indexed})", status.name, status.path.display());
    }

    if report.repo_edges.is_empty() {
        println!("\nNo cross-repo imports resolved between the configured repos.");
        return Ok(());
    }

    println!("\nCross-repo dependencies:");
    for e in &report.repo_edges {
        println!(
            "  {} -> {} ({} edge(s))",
            e.from_repo, e.to_repo, e.edge_count
        );
    }

    println!("\nImport sites:");
    for e in &report.edges {
        println!(
            "  {} :: {} -> {} :: {} ({})",
            e.from_repo,
            e.from_file.display(),
            e.to_repo,
            e.to_file.display(),
            e.import_path
        );
    }
    Ok(())
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_blast_radius(workspace: &Path, repo: &str, file: &Path) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    warn_resolution_blind_spots(&repos);
    let Some(target_repo) = repos.iter().find(|r| r.name == repo) else {
        anyhow::bail!("no repo named {repo:?} in {}", workspace.display());
    };

    let target_file = if file.is_absolute() {
        file.to_path_buf()
    } else {
        target_repo.path.join(file)
    };
    let target_file = target_file.canonicalize().unwrap_or(target_file);

    let importers = repowise_workspace::workspace_blast_radius(&repos, repo, &target_file);
    if importers.is_empty() {
        println!("No cross-repo importers found for this file.");
        return Ok(());
    }
    for e in &importers {
        println!(
            "{} :: {} ({})",
            e.from_repo,
            e.from_file.display(),
            e.import_path
        );
    }
    Ok(())
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
/// What a conformance run concluded, and whether it was in a position
/// to conclude anything.
///
/// The third variant is why this is an enum rather than a bool. "No
/// cycles found" and "no edges to check" produce the same empty cycle
/// list, and reporting both as a pass is a **false pass**: a resolver
/// pointed at a workspace of languages it doesn't cover (see
/// `repowise_graph::cross_repo::MODULE_MAP_LANGUAGES`) finds nothing
/// every time, and a CI gate that greens on that is worse than no gate,
/// because it looks like coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConformanceVerdict {
    /// Edges were resolved and no cycle exists among them. A real pass.
    Pass,
    /// At least one cycle. Fails the gate.
    Fail,
    /// Nothing resolvable to check. Neither a pass nor a failure.
    Unverified,
}

impl ConformanceVerdict {
    fn label(&self) -> &'static str {
        match self {
            ConformanceVerdict::Pass => "pass",
            ConformanceVerdict::Fail => "fail",
            ConformanceVerdict::Unverified => "unverified",
        }
    }

    /// Exit code for this verdict.
    ///
    /// `Unverified` exits non-zero alongside `Fail`. A gate that can't
    /// see anything must not report success -- the whole point of
    /// wiring this into CI is that a green run means something was
    /// actually checked. Someone who genuinely wants an unresolvable
    /// workspace to pass can say so with `--allow-unverified`, which
    /// makes that an explicit, reviewable choice rather than a silent
    /// default.
    fn exit_code(&self, allow_unverified: bool) -> i32 {
        match self {
            ConformanceVerdict::Pass => 0,
            ConformanceVerdict::Unverified if allow_unverified => 0,
            _ => 1,
        }
    }
}

/// Decide the verdict from the metrics the architecture pass already
/// computed.
///
/// Built on `workspace_metrics` rather than `detect_workspace_cycles`
/// alone so this command and `workspace-metrics` cannot disagree about
/// whether a workspace has cycles, and so the resolvability-language
/// signal is shared rather than re-derived.
fn conformance_verdict(m: &repowise_workspace::WorkspaceMetrics) -> ConformanceVerdict {
    if !m.cyclic_core.is_empty() {
        return ConformanceVerdict::Fail;
    }
    if m.edge_count == 0 {
        return ConformanceVerdict::Unverified;
    }
    ConformanceVerdict::Pass
}

fn render_conformance(
    m: &repowise_workspace::WorkspaceMetrics,
    verdict: ConformanceVerdict,
) -> String {
    let mut out = String::new();

    if !m.unindexed_repos.is_empty() {
        out.push_str(&format!(
            "{} repo(s) could not be read and contributed no edges, so any cycle\n\
             through them is invisible to this check:\n",
            m.unindexed_repos.len()
        ));
        for name in &m.unindexed_repos {
            out.push_str(&format!("  {name} -- run `repowise update` in it\n"));
        }
        out.push('\n');
    }

    match verdict {
        ConformanceVerdict::Fail => {
            out.push_str(&format!(
                "FAIL: {} circular cross-repo dependency group(s) found.\n",
                m.cyclic_core.len()
            ));
            for cycle in &m.cyclic_core {
                let mut names = cycle.clone();
                names.sort();
                out.push_str(&format!("  {}\n", names.join(" <-> ")));
            }
            out.push_str("\nA workspace's repo-level dependency graph should form a DAG.\n");
        }
        ConformanceVerdict::Pass => {
            out.push_str(&format!(
                "PASS: {} cross-repo dependency edge(s) checked, no cycles.\n",
                m.edge_count
            ));
        }
        ConformanceVerdict::Unverified => {
            out.push_str(
                "UNVERIFIED: no cross-repo dependency edges were resolved, so there was\n\
                 nothing to check for cycles. This is NOT a pass -- an empty graph and a\n\
                 clean graph look identical here.\n",
            );
            out.push_str(&format!("  reason: {}\n", m.confidence.explanation()));
            out.push_str(
                "  Pass --allow-unverified to treat this as success in CI, deliberately.\n",
            );
        }
    }

    out.push_str(
        "\nCross-repo resolution covers Rust, Python, Java, Kotlin, Scala, Go, C#, and\n\
         PHP; other languages' imports are left unresolved, so a clean result bounds\n\
         what was checked, not what exists.\n",
    );
    out
}

fn conformance_json(
    m: &repowise_workspace::WorkspaceMetrics,
    verdict: ConformanceVerdict,
) -> serde_json::Value {
    serde_json::json!({
        "verdict": verdict.label(),
        "cycles": m.cyclic_core,
        "edges_checked": m.edge_count,
        "repo_count": m.repo_count,
        "unindexed_repos": m.unindexed_repos,
        "confidence": {
            "level": m.confidence.label(),
            "explanation": m.confidence.explanation(),
        },
        "resolution_caveat":
            "cross-repo import resolution covers Rust, Python, Java, Kotlin, Scala, Go, \
             C#, and PHP; a clean result bounds what was checked, not what exists",
    })
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
fn cmd_workspace_conformance(
    workspace: &Path,
    json: bool,
    allow_unverified: bool,
) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    warn_resolution_blind_spots(&repos);
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let metrics = repowise_workspace::workspace_metrics(&repos);
    let verdict = conformance_verdict(&metrics);

    // This command gates CI, so "is this answer trustworthy" is
    // load-bearing rather than cosmetic. A workspace assembled from
    // committed artifacts can be answering about commits nobody has
    // checked out, and a clean verdict over stale inputs is exactly the
    // reassuring-but-wrong result a gate must not produce silently.
    let drifted: Vec<String> = repos
        .iter()
        .filter_map(|r| {
            let status = repowise_workspace::repo_status(r);
            (status.stale == Some(true)).then(|| status.name.clone())
        })
        .collect();
    if !json && !drifted.is_empty() {
        eprintln!(
            "note: {} of {} repo(s) have an index that has drifted from their checkout: {}. \
             Findings may not describe the current code -- re-export or re-index those repos.",
            drifted.len(),
            repos.len(),
            drifted.join(", ")
        );
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&conformance_json(&metrics, verdict))?
        );
    } else {
        print!("{}", render_conformance(&metrics, verdict));
    }

    let code = verdict.exit_code(allow_unverified);
    if code != 0 {
        // Exit directly rather than returning Err: the report above is
        // the output, and anyhow would print a second "Error:" line
        // that adds nothing.
        std::process::exit(code);
    }
    Ok(())
}

/// Render the architecture-metrics report.
///
/// Split from `cmd_workspace_metrics` so the wording is testable
/// without a workspace on disk.
///
/// Leads with the confidence caveat whenever the graph wasn't fully
/// resolvable. Cross-repo resolution here covers Rust, Python, Java,
/// Kotlin, Scala, Go, C#, and PHP, so a workspace of e.g. only
/// TypeScript services resolves zero edges and would otherwise be
/// reported as perfectly decoupled -- the best possible score for a
/// system nobody measured. That warning has to come before the numbers
/// it invalidates, not after.
fn render_metrics(m: &repowise_workspace::WorkspaceMetrics) -> String {
    use repowise_workspace::Confidence;
    let mut out = String::new();

    if m.confidence != Confidence::Resolved {
        out.push_str(&format!(
            "WARNING [{}]: {}\n\n",
            m.confidence.label(),
            m.confidence.explanation()
        ));
    }

    if !m.unindexed_repos.is_empty() {
        out.push_str(&format!(
            "{} repo(s) could not be read and contributed no edges in either\n\
             direction -- every number below is a floor, not a finding:\n",
            m.unindexed_repos.len()
        ));
        for name in &m.unindexed_repos {
            out.push_str(&format!("  {name} -- run `repowise update` in it\n"));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "Workspace architecture metrics\n  repos: {}\n  cross-repo dependency edges: {}\n",
        m.repo_count, m.edge_count
    ));
    out.push_str(&format!(
        "  propagation cost: {:.1}% (share of repo pairs where one can reach the other)\n",
        m.propagation_cost * 100.0
    ));

    match m.complexity_score {
        Some(score) => out.push_str(&format!(
            "  complexity score: {:.1}/10 ({})\n",
            score,
            repowise_workspace::WorkspaceMetrics::SCALE
        )),
        None => out.push_str(
            "  complexity score: not reported -- nothing measurable to score.\n\
             \x20   A score here would be the best possible number, earned by being\n\
             \x20   unmeasurable rather than by being well-structured.\n",
        ),
    }

    if m.cyclic_core.is_empty() {
        out.push_str("\nNo circular dependencies between repos.\n");
    } else {
        out.push_str(&format!(
            "\nCyclic core -- {} repo(s) in {} cycle(s):\n",
            m.repos_in_cyclic_core,
            m.cyclic_core.len()
        ));
        for cycle in &m.cyclic_core {
            let mut names = cycle.clone();
            names.sort();
            out.push_str(&format!("  {}\n", names.join(" <-> ")));
        }
    }

    out.push_str(
        "\nStructural edges only; co-change is excluded deliberately -- it moves with\n\
         how a team worked that quarter, and this score describes structure.\n\
         Cross-repo resolution covers Rust, Python, Java, Kotlin, Scala, Go, C#, and\n\
         PHP: every other language's imports are left unresolved, so these numbers\n\
         are a lower bound on real coupling.\n",
    );

    out
}

fn metrics_json(m: &repowise_workspace::WorkspaceMetrics) -> serde_json::Value {
    serde_json::json!({
        "repo_count": m.repo_count,
        "edge_count": m.edge_count,
        "propagation_cost": m.propagation_cost,
        "complexity_score": m.complexity_score,
        "complexity_scale": repowise_workspace::WorkspaceMetrics::SCALE,
        "cyclic_core": m.cyclic_core,
        "repos_in_cyclic_core": m.repos_in_cyclic_core,
        "unindexed_repos": m.unindexed_repos,
        "confidence": {
            "level": m.confidence.label(),
            "explanation": m.confidence.explanation(),
        },
        "excludes": ["co-change (behavioral, not structural)"],
        "resolution_caveat":
            "cross-repo import resolution covers Rust, Python, Java, Kotlin, Scala, Go, \
             C#, and PHP; other languages' imports are unresolved, so these numbers are \
             a lower bound on real coupling",
    })
}

fn cmd_workspace_metrics(workspace: &Path, json: bool) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    warn_resolution_blind_spots(&repos);
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let metrics = repowise_workspace::workspace_metrics(&repos);
    if json {
        println!("{}", serde_json::to_string_pretty(&metrics_json(&metrics))?);
    } else {
        print!("{}", render_metrics(&metrics));
    }
    Ok(())
}

/// Render the contract diagnostics report.
///
/// Split from `cmd_workspace_diagnostics` so the wording -- which is
/// most of this feature's value -- is testable without a workspace on
/// disk.
///
/// Leads with unindexed repos when there are any. That ordering is the
/// point of the whole command: every other number below is a floor
/// rather than a finding while a repo is missing from the scan, and
/// burying that under the counts would let someone read a confidently
/// wrong conclusion off the top of the output.
fn render_diagnostics(diag: &repowise_workspace::ContractDiagnostics) -> String {
    let mut out = String::new();
    let unindexed = diag.unindexed_repos();

    if !unindexed.is_empty() {
        out.push_str(&format!(
            "WARNING: {} of {} repo(s) could not be read, and contributed no routes\n\
             and no calls to anything below. Every count in this report is a floor,\n\
             not a finding, until these are indexed:\n",
            unindexed.len(),
            diag.repos.len()
        ));
        for name in &unindexed {
            out.push_str(&format!("  {name} -- run `repowise update` in it\n"));
        }
        out.push('\n');
    }

    out.push_str("Per-repo endpoints found:\n");
    for repo in &diag.repos {
        if repo.indexed {
            out.push_str(&format!(
                "  {:<24} {} producer route(s), {} consumer call(s)\n",
                repo.repo, repo.producers, repo.consumers
            ));
        } else {
            out.push_str(&format!("  {:<24} not indexed -- not scanned\n", repo.repo));
        }
    }

    out.push_str(&format!(
        "\nMatched cross-repo contracts: {}\n",
        diag.matches
    ));

    let by_reason = diag.unmatched_by_reason();
    if by_reason.is_empty() {
        out.push_str("\nEvery consumer call matched a producer in another repo.\n");
    } else {
        out.push_str("\nConsumer calls with no cross-repo contract, by reason:\n");
        for (reason, count) in by_reason {
            out.push_str(&format!(
                "  {:<22} {:>4}  -- {}\n",
                reason.label(),
                count,
                reason.explanation()
            ));
        }
        out.push_str("\nDetail:\n");
        for u in &diag.unmatched_consumers {
            out.push_str(&format!(
                "  [{}] {} ({} :: {})\n",
                u.reason.label(),
                u.call.path,
                u.call.repo,
                u.call.file.display()
            ));
        }
    }

    if diag.orphan_producers.is_empty() {
        out.push_str("\nNo orphan producer routes.\n");
    } else {
        out.push_str(&format!(
            "\nProducer routes nothing in this workspace calls ({}):\n\
             \x20 Either dead surface, or a consumer idiom this scan doesn't recognize --\n\
             \x20 this scan can't tell which, so neither is claimed.\n",
            diag.orphan_producers.len()
        ));
        for o in &diag.orphan_producers {
            out.push_str(&format!(
                "  {} ({} :: {})\n",
                o.route.path,
                o.route.repo,
                o.route.file.display()
            ));
        }
    }

    out
}

/// The report as JSON, hand-built rather than derived.
///
/// `ContractDiagnostics` deliberately doesn't derive `Serialize`:
/// `repowise-workspace` is a library crate with no serde dependency,
/// and the JSON shape here is a CLI output contract, which is a
/// different thing from the in-memory type and should be free to
/// diverge from it.
fn diagnostics_json(diag: &repowise_workspace::ContractDiagnostics) -> serde_json::Value {
    serde_json::json!({
        "repos": diag.repos.iter().map(|r| serde_json::json!({
            "repo": r.repo,
            "indexed": r.indexed,
            "producers": r.producers,
            "consumers": r.consumers,
        })).collect::<Vec<_>>(),
        "unindexed_repos": diag.unindexed_repos(),
        "matches": diag.matches,
        "unmatched_by_reason": diag.unmatched_by_reason().into_iter().map(|(r, n)| serde_json::json!({
            "reason": r.label(),
            "count": n,
            "explanation": r.explanation(),
        })).collect::<Vec<_>>(),
        "unmatched_consumers": diag.unmatched_consumers.iter().map(|u| serde_json::json!({
            "reason": u.reason.label(),
            "path": u.call.path,
            "repo": u.call.repo,
            "file": u.call.file.display().to_string(),
            "method": u.call.method,
        })).collect::<Vec<_>>(),
        "orphan_producers": diag.orphan_producers.iter().map(|o| serde_json::json!({
            "path": o.route.path,
            "repo": o.route.repo,
            "file": o.route.file.display().to_string(),
            "method": o.route.method,
        })).collect::<Vec<_>>(),
    })
}

fn cmd_workspace_diagnostics(workspace: &Path, json: bool) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let diag = repowise_workspace::workspace_diagnostics(&repos);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&diagnostics_json(&diag))?
        );
    } else {
        print!("{}", render_diagnostics(&diag));
    }
    Ok(())
}

/// The report as JSON, hand-built for the same reason
/// `diagnostics_json` is: `repowise-workspace`'s in-memory types aren't
/// the CLI's output contract.
fn contract_changes_json(
    report: &repowise_workspace::ContractsReport,
    broken: &[repowise_workspace::BrokenContract],
) -> serde_json::Value {
    serde_json::json!({
        "matches": report.matches.iter().map(|m| serde_json::json!({
            "path": m.path,
            "producer_repo": m.producer_repo,
            "producer_file": m.producer_file.display().to_string(),
            "consumer_repo": m.consumer_repo,
            "consumer_file": m.consumer_file.display().to_string(),
        })).collect::<Vec<_>>(),
        "unmatched_consumers": report.unmatched_consumers.iter().map(|c| serde_json::json!({
            "path": c.path,
            "repo": c.repo,
            "file": c.file.display().to_string(),
        })).collect::<Vec<_>>(),
        "broken": broken.iter().map(|b| serde_json::json!({
            "path": b.key.path,
            "consumer_repo": b.key.consumer_repo,
            "consumer_file": b.key.consumer_file.display().to_string(),
            "previous_producer_repo": b.key.producer_repo,
            "reason": b.reason.map(|r| r.label()),
        })).collect::<Vec<_>>(),
    })
}

/// See `repowise-workspace`'s own docs for the workspace TOML format.
///
/// Persists a snapshot of this run's matches to
/// `.repowise-workspace/contracts.json` (sibling to `workspace`, via
/// `repowise_workspace::workspace_state_dir`) and diffs against
/// whatever the *previous* run left there, so a contract that used to
/// resolve and stopped is reported as broken rather than silently
/// dropping out of the match list. Every run overwrites the snapshot
/// with its own current state, the same "current run becomes the new
/// baseline" model `repowise update` uses for `.repowise/index.json`.
fn cmd_workspace_contracts(workspace: &Path, json: bool) -> anyhow::Result<()> {
    let repos = repowise_workspace::load_resolved(workspace)?;
    if repos.is_empty() {
        println!("No repos configured in {}", workspace.display());
        return Ok(());
    }
    let state_dir = repowise_workspace::workspace_state_dir(workspace);
    let (report, broken) = repowise_workspace::workspace_contract_changes(&repos, &state_dir);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&contract_changes_json(&report, &broken))?
        );
        return Ok(());
    }

    if !broken.is_empty() {
        println!(
            "BROKEN: {} contract(s) that used to resolve no longer do:",
            broken.len()
        );
        for b in &broken {
            let reason = match b.reason {
                Some(r) => r.explanation(),
                None => "the consumer call site itself is gone",
            };
            println!(
                "  {} ({} :: {}) used to resolve to {} -- {}",
                b.key.path,
                b.key.consumer_repo,
                b.key.consumer_file.display(),
                b.key.producer_repo,
                reason
            );
        }
        println!();
    }

    if report.matches.is_empty() {
        println!("No cross-repo API contracts matched.");
    } else {
        println!("Matched cross-repo API contracts:");
        for m in &report.matches {
            println!(
                "  {}: {} ({}) <- {} ({})",
                m.path,
                m.producer_repo,
                m.producer_file.display(),
                m.consumer_repo,
                m.consumer_file.display()
            );
        }
    }

    if !report.unmatched_consumers.is_empty() {
        println!("\nConsumer calls with no known producer in this workspace:");
        for c in &report.unmatched_consumers {
            println!("  {} ({} :: {})", c.path, c.repo, c.file.display());
        }
    }
    Ok(())
}

fn cmd_generate(path: &Path) -> anyhow::Result<()> {
    let Some(config) = repowise_llm::LlmConfig::from_env() else {
        anyhow::bail!(
            "LLM-assisted generation is opt-in and REPOWISE_LLM_BASE_URL isn't set. \
             Point it at an OpenAI-compatible endpoint (e.g. a self-hosted rusty_provider \
             instance) to enable it -- see README.md."
        );
    };
    let root = path.canonicalize()?;
    let index = RepoIndex::load(&root)?;

    let results = repowise_llm::generate_wiki_summaries(&index, &config);
    let written = results
        .iter()
        .filter(|r| r.status == repowise_llm::SummaryStatus::Written)
        .count();
    let missing = results
        .iter()
        .filter(|r| r.status == repowise_llm::SummaryStatus::NoWikiPage)
        .count();
    let failed = results
        .iter()
        .filter(|r| r.status == repowise_llm::SummaryStatus::Failed)
        .count();

    println!(
        "Added LLM summaries to {written} wiki page(s) under {}/.repowise/wiki",
        index.root.display()
    );
    if missing > 0 {
        println!("  {missing} file(s) skipped: no wiki page yet -- run `repowise docs` first");
    }
    if failed > 0 {
        println!("  {failed} file(s) failed (LLM call or write error)");
    }

    // Second ask in the same pass, not a second pipeline: the files are
    // already being read and the model is already being called.
    let (proposals, _) = repowise_llm::propose_decisions(&index, &config)?;
    println!(
        "Inferred {} architectural decision(s) from {} file(s) -> `repowise decisions`",
        proposals.kept, proposals.files_considered
    );
    // Every drop is reported. A pass that discards most of what the
    // model returned is a fact about that model worth seeing, and
    // reporting only the survivors would present a filtered number as
    // if it were the whole story.
    if proposals.dropped_unanchored > 0 {
        println!(
            "  {} dropped: the code they quoted isn't in the file (proposals must quote \
             verbatim, and the quote is checked)",
            proposals.dropped_unanchored
        );
    }
    if proposals.dropped_incomplete > 0 {
        println!(
            "  {} dropped: missing a title or rationale, or duplicated another proposal's \
             anchor",
            proposals.dropped_incomplete
        );
    }
    if proposals.unparseable_files > 0 {
        println!(
            "  {} file(s) contributed nothing: the model's reply wasn't the requested JSON",
            proposals.unparseable_files
        );
    }
    if proposals.failed_files > 0 {
        println!(
            "  {} file(s) contributed nothing: the LLM call failed",
            proposals.failed_files
        );
    }
    println!(
        "  These are a model's reading of the code, not anything a person wrote down. \
         Every surface that shows them labels them as inferred."
    );
    Ok(())
}

/// Load the index a read command should work from: the machine-local
/// `.repowise/index.json` by default, or a committed portable artifact
/// when `--index` names one (issue #378).
///
/// **Staleness is always reported when reading a committed artifact.**
/// A committed index is routinely behind the working tree — that is the
/// normal case, not an error — but presenting stale analysis as current
/// is the one failure mode this format could actively mislead with, so
/// ADR-0002 makes the report mandatory rather than opt-in. Unknown is
/// reported as unknown: "no commit recorded" never silently reads as
/// "up to date".
fn load_index(root: &Path, index_file: Option<&Path>) -> anyhow::Result<RepoIndex> {
    let Some(file) = index_file else {
        return RepoIndex::load(root);
    };
    let portable = repowise_core::portable::PortableIndex::load(file)?;
    let index = portable.into_anchored(root)?;

    match (&index.indexed_commit, repowise_git::head_sha(root)) {
        (Some(indexed), Some(head)) if indexed == &head => {
            eprintln!(
                "note: reading {} (built at {indexed}, matches your checkout)",
                file.display()
            );
        }
        (Some(indexed), Some(head)) => {
            eprintln!(
                "note: reading {} -- STALE: built at {indexed}, your checkout is at {head}. \
                 Findings may not match your working tree.",
                file.display()
            );
        }
        (Some(indexed), None) => {
            eprintln!(
                "note: reading {} (built at {indexed}); this directory has no git HEAD to \
                 compare against, so staleness is unknown.",
                file.display()
            );
        }
        (None, _) => {
            eprintln!(
                "note: reading {} -- it records no indexed commit, so whether it matches \
                 your checkout is unknown.",
                file.display()
            );
        }
    }
    Ok(index)
}

/// Warn when a workspace member's cross-repo imports silently cannot
/// resolve (issue #384).
///
/// Printed to stderr by every command that depends on cross-repo
/// resolution. The failure mode this guards against is specific: a
/// Rust or Go member backed only by a committed artifact, with no
/// checkout, contributes nothing to the module map because its crate /
/// module name lives in a `Cargo.toml` / `go.mod` that isn't there. The
/// result is not an error -- it is an empty edge list, which is
/// indistinguishable from "these repos genuinely don't depend on each
/// other".
/// Module paths for the files whose language derives one by reading a
/// manifest off disk -- Rust (`Cargo.toml`) and Go (`go.mod`), issue
/// #388.
///
/// The other four cross-repo languages (Python, JVM, C#, PHP) derive
/// theirs from `(file, root)` by string manipulation, so a reader can
/// always recompute them and recording them would be dead weight.
fn disk_derived_module_paths(index: &RepoIndex) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for lang in [repowise_core::Language::Rust, repowise_core::Language::Go] {
        for (module_path, file) in repowise_graph::module_map(index, lang) {
            out.push((file, module_path));
        }
    }
    out
}

fn warn_resolution_blind_spots(repos: &[repowise_workspace::ResolvedWorkspaceRepo]) {
    let blind = repowise_workspace::resolution_blind_spots(repos);
    if blind.is_empty() {
        return;
    }
    for (repo, language) in &blind {
        eprintln!(
            "warning: {repo} is backed by a portable index with no checkout at its path, \
             and contains {language} files. {language}'s cross-repo module map is derived \
             from a manifest on disk (Cargo.toml / go.mod), so this repo's imports cannot \
             resolve and will silently contribute no edges."
        );
    }
    eprintln!(
        "  Check out those repo(s), or treat cross-repo results as incomplete. Python, \
         Java/Kotlin/Scala, C#, and PHP members are unaffected -- their module paths need \
         no files on disk."
    );
}

/// How to say a workspace repo's index freshness in one clause.
///
/// Unknown is spelled out rather than omitted: a reader who sees nothing
/// will assume "fine", and "we couldn't tell" is a different answer from
/// "it matches".
fn staleness_suffix(stale: Option<bool>) -> &'static str {
    match stale {
        Some(true) => ", STALE vs its checkout",
        Some(false) => ", matches its checkout",
        None => ", freshness unknown",
    }
}

fn display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_workspace::{
        ContractDiagnostics, OrphanProducer, RepoEndpointCounts, UnmatchedConsumer, UnmatchedReason,
    };

    /// Which commands expose `--index` is a deliberate list, not an
    /// accident of which ones happened to get wired (issue #382). These
    /// two tests pin both halves of it, so adding a command without
    /// deciding which half it belongs in fails here rather than shipping.
    ///
    /// The read commands: index-derived, and correct when the answer
    /// comes from an artifact somebody else built.
    #[test]
    fn every_index_derived_read_command_accepts_an_index_override() {
        let cases: Vec<Vec<&str>> = vec![
            vec!["repowise", "overview", "."],
            vec!["repowise", "health", "."],
            vec!["repowise", "deps", "src/lib.rs", "."],
            vec!["repowise", "tour", "."],
            vec!["repowise", "search", "query", "."],
            vec!["repowise", "dead-code", "."],
            vec!["repowise", "refactor", "."],
            vec!["repowise", "security", "."],
            vec!["repowise", "hotspots", "."],
            vec!["repowise", "doc-coverage", "."],
            vec!["repowise", "decisions", "."],
        ];
        for case in cases {
            let mut argv = case.clone();
            argv.extend(["--index", "shared.portable.json"]);
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "{} should accept --index",
                case[1]
            );
        }
    }

    /// The other half. Each of these is excluded for a stated reason,
    /// and the point is that they *reject* the flag rather than
    /// accepting and ignoring it -- silently reading a different index
    /// than the caller named is the failure this whole feature exists to
    /// avoid.
    #[test]
    fn commands_that_cannot_honour_an_index_override_reject_it() {
        // `update` builds the index; `docs`/`generate` write output
        // derived from it, and writing artifact-derived content into the
        // repo is exactly where staleness stops being cosmetic;
        // `status`/`doctor` report on the *local* index, which is their
        // entire subject; `export` reading an export is circular.
        for cmd in ["update", "docs", "generate", "status", "doctor", "export"] {
            let argv = vec!["repowise", cmd, ".", "--index", "shared.portable.json"];
            assert!(
                Cli::try_parse_from(&argv).is_err(),
                "{cmd} should reject --index rather than silently ignoring it"
            );
        }
    }

    fn candidate(
        name: &str,
        line: usize,
        confidence: DeadCodeConfidence,
        risk_factors: Vec<String>,
    ) -> DeadCodeCandidate {
        DeadCodeCandidate {
            file: PathBuf::from("/repo/src/lib.rs"),
            symbol: name.to_string(),
            line,
            confidence,
            risk_factors,
        }
    }

    #[test]
    fn parses_confidence_flag_case_insensitively() {
        assert_eq!(
            parse_min_confidence("HIGH").unwrap(),
            DeadCodeConfidence::High
        );
        assert_eq!(
            parse_min_confidence("medium").unwrap(),
            DeadCodeConfidence::Medium
        );
        assert!(parse_min_confidence("bogus").is_err());
    }

    #[test]
    fn renders_candidates_with_paths_relative_to_the_repo_root() {
        let out = render_dead_code(
            vec![candidate("unused_fn", 12, DeadCodeConfidence::High, vec![])],
            Path::new("/repo"),
            DeadCodeConfidence::Low,
            50,
        );
        assert!(out.contains("src/lib.rs:12"), "{out}");
        assert!(out.contains("unused_fn"), "{out}");
        assert!(out.contains("high"), "{out}");
    }

    #[test]
    fn filters_out_candidates_below_the_requested_tier() {
        let out = render_dead_code(
            vec![
                candidate(
                    "low_one",
                    1,
                    DeadCodeConfidence::Low,
                    vec!["ambiguous".into()],
                ),
                candidate("high_one", 2, DeadCodeConfidence::High, vec![]),
            ],
            Path::new("/repo"),
            DeadCodeConfidence::High,
            50,
        );
        assert!(out.contains("high_one"), "{out}");
        assert!(!out.contains("low_one"), "{out}");
        assert!(out.contains("1 candidate(s)"), "{out}");
    }

    #[test]
    fn reports_risk_factors_for_sub_high_candidates() {
        let out = render_dead_code(
            vec![candidate(
                "maybe_dead",
                3,
                DeadCodeConfidence::Medium,
                vec!["another symbol shares this name".into()],
            )],
            Path::new("/repo"),
            DeadCodeConfidence::Low,
            50,
        );
        assert!(out.contains("another symbol shares this name"), "{out}");
    }

    #[test]
    fn truncates_to_the_limit_but_still_reports_the_full_count() {
        let candidates: Vec<_> = (0..5)
            .map(|i| candidate(&format!("fn_{i}"), i, DeadCodeConfidence::High, vec![]))
            .collect();
        let out = render_dead_code(candidates, Path::new("/repo"), DeadCodeConfidence::Low, 2);
        assert!(out.contains("5 candidate(s)"), "{out}");
        assert!(out.contains("3 more not shown"), "{out}");
        assert!(out.contains("fn_1"), "{out}");
        assert!(!out.contains("fn_4"), "{out}");
    }

    #[test]
    fn empty_result_is_reported_as_a_clean_bill_not_an_error() {
        let out = render_dead_code(vec![], Path::new("/repo"), DeadCodeConfidence::High, 50);
        assert!(out.contains("no candidates"), "{out}");
    }

    fn risk(score: f64, files: usize) -> repowise_git::ChangeRisk {
        repowise_git::ChangeRisk {
            revspec: "HEAD".to_string(),
            lines_added: 40,
            lines_deleted: 5,
            files_touched: files,
            subsystems_touched: 2,
            concentration: 0.75,
            author: "Ada".to_string(),
            author_prior_commits: 12,
            score,
        }
    }

    #[test]
    fn bands_risk_scores_into_words() {
        assert_eq!(risk_band(9.0), "high");
        assert_eq!(risk_band(7.0), "high");
        assert_eq!(risk_band(5.5), "moderate");
        assert_eq!(risk_band(4.0), "moderate");
        assert_eq!(risk_band(1.2), "low");
    }

    #[test]
    fn renders_the_diff_shape_and_author_experience() {
        let out = render_risk(&risk(5.5, 3));
        assert!(out.contains("5.5/10"), "{out}");
        assert!(out.contains("moderate"), "{out}");
        assert!(out.contains("3 file(s)"), "{out}");
        assert!(out.contains("+40 / -5 line(s)"), "{out}");
        assert!(out.contains("Ada"), "{out}");
        assert!(out.contains("12 prior commit(s)"), "{out}");
    }

    #[test]
    fn states_that_the_score_is_a_heuristic_not_a_probability() {
        let out = render_risk(&risk(2.0, 1));
        assert!(out.contains("heuristic"), "{out}");
    }

    #[test]
    fn reports_the_revspec_it_actually_scored() {
        let mut r = risk(3.0, 1);
        r.revspec = "main..feature".to_string();
        let out = render_risk(&r);
        assert!(out.contains("main..feature"), "{out}");
    }

    #[test]
    fn status_reports_a_missing_index_as_a_state_not_an_error() {
        let report = StatusReport::default();
        let out = render_status(&report, Path::new("/repo"), false);
        assert!(out.contains("index: none"), "{out}");
        assert!(out.contains("repowise init"), "{out}");
    }

    #[test]
    fn status_reports_a_clean_index_as_up_to_date() {
        let report = StatusReport {
            indexed: Some(IndexedStatus {
                file_count: 7,
                stale: vec![],
                missing: vec![],
            }),
            wiki_pages: 3,
        };
        let out = render_status(&report, Path::new("/repo"), false);
        assert!(out.contains("7 file(s) indexed"), "{out}");
        assert!(out.contains("up to date"), "{out}");
        assert!(out.contains("3 page(s)"), "{out}");
    }

    #[test]
    fn status_counts_stale_and_missing_separately() {
        let report = StatusReport {
            indexed: Some(IndexedStatus {
                file_count: 5,
                stale: vec![PathBuf::from("/repo/a.rs"), PathBuf::from("/repo/b.rs")],
                missing: vec![PathBuf::from("/repo/gone.rs")],
            }),
            ..Default::default()
        };
        let out = render_status(&report, Path::new("/repo"), false);
        assert!(out.contains("2 modified, 1 missing"), "{out}");
        assert!(out.contains("repowise update"), "{out}");
        assert!(
            !out.contains("a.rs"),
            "non-verbose should not list files: {out}"
        );
    }

    #[test]
    fn status_verbose_lists_the_individual_files() {
        let report = StatusReport {
            indexed: Some(IndexedStatus {
                file_count: 5,
                stale: vec![PathBuf::from("/repo/a.rs")],
                missing: vec![PathBuf::from("/repo/gone.rs")],
            }),
            ..Default::default()
        };
        let out = render_status(&report, Path::new("/repo"), true);
        assert!(out.contains("modified  a.rs"), "{out}");
        assert!(out.contains("missing   gone.rs"), "{out}");
    }

    #[test]
    fn status_states_the_new_file_blind_spot() {
        let report = StatusReport {
            indexed: Some(IndexedStatus {
                file_count: 1,
                stale: vec![],
                missing: vec![],
            }),
            ..Default::default()
        };
        let out = render_status(&report, Path::new("/repo"), false);
        assert!(
            out.contains("created since indexing aren't detected"),
            "{out}"
        );
    }

    #[test]
    fn bus_factor_phrasing_cannot_be_read_as_a_quality_score() {
        // A bare "1" invites "one owner, tidy" -- the opposite of the meaning.
        let one = render_bus_factor(1);
        assert!(one.contains("one author wrote most"), "{one}");

        let three = render_bus_factor(3);
        assert!(three.contains("3 authors between them"), "{three}");

        let none = render_bus_factor(0);
        assert!(none.contains("n/a"), "{none}");
        assert!(none.contains("no blameable lines"), "{none}");
    }

    fn counts(repo: &str, producers: usize, consumers: usize, indexed: bool) -> RepoEndpointCounts {
        RepoEndpointCounts {
            repo: repo.to_string(),
            producers,
            consumers,
            indexed,
        }
    }

    fn call(repo: &str, path: &str) -> repowise_workspace::ConsumerCall {
        repowise_workspace::ConsumerCall {
            repo: repo.to_string(),
            file: PathBuf::from("app.js"),
            method: None,
            path: path.to_string(),
        }
    }

    /// An unindexed repo must be the FIRST thing the report says.
    /// Every count below it is a floor rather than a finding while a
    /// repo is missing from the scan, and burying that under the
    /// numbers is how someone reads a confidently wrong conclusion off
    /// the top of the output.
    #[test]
    fn diagnostics_leads_with_unread_repos() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 3, 0, true), counts("web", 0, 0, false)],
            matches: 0,
            unmatched_consumers: Vec::new(),
            orphan_producers: Vec::new(),
        };
        let out = render_diagnostics(&diag);
        let warn = out
            .find("WARNING")
            .expect("unread repos must be called out");
        let counts_at = out.find("Per-repo endpoints").unwrap();
        assert!(warn < counts_at, "the warning must come first:\n{out}");
        assert!(out.contains("floor"), "{out}");
        assert!(out.contains("repowise update"), "{out}");
        assert!(
            out.contains("web") && out.contains("not indexed -- not scanned"),
            "an unread repo's row must not read as '0 producers, 0 consumers':\n{out}"
        );
    }

    /// A clean workspace must not print the warning at all -- a banner
    /// that shows up unconditionally is one nobody reads.
    #[test]
    fn diagnostics_stays_quiet_when_every_repo_was_read() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 2, 0, true), counts("web", 0, 2, true)],
            matches: 2,
            unmatched_consumers: Vec::new(),
            orphan_producers: Vec::new(),
        };
        let out = render_diagnostics(&diag);
        assert!(!out.contains("WARNING"), "{out}");
        assert!(out.contains("Every consumer call matched"), "{out}");
        assert!(out.contains("No orphan producer routes"), "{out}");
    }

    #[test]
    fn diagnostics_groups_unmatched_consumers_by_reason_with_an_explanation() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 1, 2, true)],
            matches: 0,
            unmatched_consumers: vec![
                UnmatchedConsumer {
                    call: call("api", "/api/local"),
                    reason: UnmatchedReason::SameRepoOnly,
                },
                UnmatchedConsumer {
                    call: call("api", "/api/gone"),
                    reason: UnmatchedReason::NoProducerAnywhere,
                },
            ],
            orphan_producers: Vec::new(),
        };
        let out = render_diagnostics(&diag);
        assert!(out.contains("same-repo-only"), "{out}");
        assert!(out.contains("no-producer-anywhere"), "{out}");
        // The explanation is the point: a bare count of "2 unmatched"
        // is exactly the undifferentiated number this command exists
        // to break apart.
        assert!(out.contains("not a cross-repo contract"), "{out}");
        assert!(out.contains("pattern table doesn't recognize"), "{out}");
    }

    #[test]
    fn diagnostics_refuses_to_call_orphan_producers_dead() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 1, 0, true)],
            matches: 0,
            unmatched_consumers: Vec::new(),
            orphan_producers: vec![OrphanProducer {
                route: repowise_workspace::ProducerRoute {
                    repo: "api".to_string(),
                    file: PathBuf::from("routes.rs"),
                    method: None,
                    path: "/api/unused".to_string(),
                },
            }],
        };
        let out = render_diagnostics(&diag);
        assert!(out.contains("/api/unused"), "{out}");
        assert!(
            out.contains("Either dead surface, or a consumer idiom this scan doesn't recognize"),
            "an orphan route is ambiguous and the report must not resolve it:\n{out}"
        );
    }

    #[test]
    fn diagnostics_json_carries_the_reasons_not_just_the_counts() {
        let diag = ContractDiagnostics {
            repos: vec![counts("api", 0, 1, true), counts("web", 0, 0, false)],
            matches: 0,
            unmatched_consumers: vec![UnmatchedConsumer {
                call: call("api", "/api/gone"),
                reason: UnmatchedReason::NoProducerAnywhere,
            }],
            orphan_producers: Vec::new(),
        };
        let v = diagnostics_json(&diag);
        assert_eq!(v["unindexed_repos"][0], "web");
        assert_eq!(
            v["unmatched_by_reason"][0]["reason"],
            "no-producer-anywhere"
        );
        assert!(v["unmatched_by_reason"][0]["explanation"]
            .as_str()
            .unwrap()
            .contains("external"));
        assert_eq!(v["unmatched_consumers"][0]["path"], "/api/gone");
    }

    fn metrics(
        confidence: repowise_workspace::Confidence,
        score: Option<f64>,
        cyclic: Vec<Vec<String>>,
    ) -> repowise_workspace::WorkspaceMetrics {
        repowise_workspace::WorkspaceMetrics {
            repo_count: 3,
            edge_count: 2,
            propagation_cost: 0.55,
            repos_in_cyclic_core: cyclic.iter().flatten().count(),
            cyclic_core: cyclic,
            complexity_score: score,
            unindexed_repos: Vec::new(),
            confidence,
        }
    }

    /// The whole reason the score is an Option. A workspace this port
    /// can't resolve must not be handed the best possible number for
    /// having been unmeasurable.
    #[test]
    fn metrics_withholds_the_score_when_nothing_was_measurable() {
        let out = render_metrics(&metrics(
            repowise_workspace::Confidence::NoResolvableLanguage,
            None,
            Vec::new(),
        ));
        assert!(out.contains("not reported"), "{out}");
        assert!(
            out.contains("unmeasurable rather than by being well-structured"),
            "{out}"
        );
        let warn = out.find("WARNING").expect("must warn");
        let numbers = out.find("Workspace architecture metrics").unwrap();
        assert!(
            warn < numbers,
            "the caveat must precede the numbers:\n{out}"
        );
    }

    #[test]
    fn metrics_states_the_scale_so_a_number_is_not_read_as_a_grade() {
        let out = render_metrics(&metrics(
            repowise_workspace::Confidence::Resolved,
            Some(4.2),
            Vec::new(),
        ));
        assert!(out.contains("4.2/10"), "{out}");
        assert!(out.contains("lower is better"), "{out}");
        assert!(
            !out.contains("WARNING"),
            "a resolved graph needs no caveat banner:\n{out}"
        );
    }

    #[test]
    fn metrics_always_states_what_was_excluded_and_what_could_not_resolve() {
        let out = render_metrics(&metrics(
            repowise_workspace::Confidence::Resolved,
            Some(2.0),
            Vec::new(),
        ));
        // Present even on the happy path: these bound what the number
        // means, and a reader who only ever sees clean output would
        // otherwise never learn the limits.
        assert!(out.contains("co-change is excluded"), "{out}");
        assert!(out.contains("PHP"), "{out}");
        assert!(out.contains("lower bound on real coupling"), "{out}");
    }

    #[test]
    fn metrics_lists_each_cycle() {
        let out = render_metrics(&metrics(
            repowise_workspace::Confidence::Resolved,
            Some(8.0),
            vec![vec!["b".to_string(), "a".to_string()]],
        ));
        assert!(out.contains("Cyclic core"), "{out}");
        assert!(
            out.contains("a <-> b"),
            "cycle members are sorted for stable output:\n{out}"
        );
    }

    #[test]
    fn metrics_json_carries_the_confidence_and_the_caveat() {
        let v = metrics_json(&metrics(
            repowise_workspace::Confidence::NoResolvableLanguage,
            None,
            Vec::new(),
        ));
        assert_eq!(v["confidence"]["level"], "no-resolvable-language");
        assert!(v["complexity_score"].is_null(), "withheld, not zero or ten");
        assert!(v["resolution_caveat"].as_str().unwrap().contains("PHP"));
        assert_eq!(
            v["complexity_scale"],
            repowise_workspace::WorkspaceMetrics::SCALE
        );
    }

    fn conf_metrics(
        edges: usize,
        cycles: Vec<Vec<String>>,
        confidence: repowise_workspace::Confidence,
    ) -> repowise_workspace::WorkspaceMetrics {
        repowise_workspace::WorkspaceMetrics {
            repo_count: 3,
            edge_count: edges,
            propagation_cost: 0.5,
            repos_in_cyclic_core: cycles.iter().flatten().count(),
            cyclic_core: cycles,
            complexity_score: Some(2.0),
            unindexed_repos: Vec::new(),
            confidence,
        }
    }

    #[test]
    fn conformance_fails_on_a_cycle() {
        let m = conf_metrics(
            4,
            vec![vec!["b".to_string(), "a".to_string()]],
            repowise_workspace::Confidence::Resolved,
        );
        let v = conformance_verdict(&m);
        assert_eq!(v, ConformanceVerdict::Fail);
        assert_eq!(v.exit_code(false), 1, "a cycle must gate CI");
        assert_eq!(
            v.exit_code(true),
            1,
            "--allow-unverified must not excuse a real cycle"
        );
        let out = render_conformance(&m, v);
        assert!(out.contains("FAIL"), "{out}");
        assert!(out.contains("a <-> b"), "{out}");
    }

    #[test]
    fn conformance_passes_when_real_edges_were_checked() {
        let m = conf_metrics(4, Vec::new(), repowise_workspace::Confidence::Resolved);
        let v = conformance_verdict(&m);
        assert_eq!(v, ConformanceVerdict::Pass);
        assert_eq!(v.exit_code(false), 0);
        let out = render_conformance(&m, v);
        assert!(
            out.contains("4 cross-repo dependency edge(s) checked"),
            "a pass must say how much was checked, or it means nothing:\n{out}"
        );
    }

    /// The false pass this command exists to prevent. An empty graph
    /// and a clean graph produce the same empty cycle list, and a CI
    /// gate that greens on the former looks like coverage while
    /// providing none.
    #[test]
    fn conformance_will_not_call_an_unresolvable_workspace_a_pass() {
        let m = conf_metrics(
            0,
            Vec::new(),
            repowise_workspace::Confidence::NoResolvableLanguage,
        );
        let v = conformance_verdict(&m);
        assert_eq!(v, ConformanceVerdict::Unverified);
        assert_eq!(
            v.exit_code(false),
            1,
            "unverified must not exit 0 by default"
        );
        assert_eq!(v.exit_code(true), 0, "but it can be opted into explicitly");
        let out = render_conformance(&m, v);
        assert!(out.contains("UNVERIFIED"), "{out}");
        assert!(out.contains("NOT a pass"), "{out}");
        assert!(out.contains("--allow-unverified"), "{out}");
    }

    #[test]
    fn conformance_warns_that_unread_repos_hide_cycles() {
        let mut m = conf_metrics(2, Vec::new(), repowise_workspace::Confidence::Resolved);
        m.unindexed_repos = vec!["ghost".to_string()];
        let out = render_conformance(&m, conformance_verdict(&m));
        assert!(
            out.contains("any cycle\nthrough them is invisible"),
            "an unread repo can hide the very cycle this gate looks for:\n{out}"
        );
    }

    #[test]
    fn conformance_json_carries_the_verdict_and_what_was_checked() {
        let m = conf_metrics(
            0,
            Vec::new(),
            repowise_workspace::Confidence::NoResolvableLanguage,
        );
        let v = conformance_json(&m, conformance_verdict(&m));
        assert_eq!(v["verdict"], "unverified");
        assert_eq!(v["edges_checked"], 0);
        assert!(v["resolution_caveat"].as_str().unwrap().contains("PHP"));
    }

    fn rec(
        kind: repowise_distill::ledger::Kind,
        program: &str,
        raw: usize,
        kept: usize,
        detail: &str,
    ) -> repowise_distill::ledger::Record {
        repowise_distill::ledger::Record {
            at: 1_700_000_000,
            kind,
            program: program.to_string(),
            raw_bytes: raw,
            kept_bytes: kept,
            detail: detail.to_string(),
            exit_code: Some(0),
        }
    }

    /// An empty report must not read as "distillation saved you
    /// nothing". Nothing was measured, which is a different claim.
    #[test]
    fn saved_with_no_records_says_nothing_was_measured() {
        let out = render_saved(&[], "program");
        assert!(out.contains("No distillations recorded"), "{out}");
        assert!(
            out.contains("nothing to measure"),
            "must distinguish 'saved nothing' from 'measured nothing':\n{out}"
        );
    }

    #[test]
    fn saved_totals_measured_bytes_and_labels_tokens_as_approximate() {
        let records = vec![
            rec(
                repowise_distill::ledger::Kind::Distilled,
                "cargo",
                4000,
                400,
                "",
            ),
            rec(
                repowise_distill::ledger::Kind::Distilled,
                "pytest",
                2000,
                200,
                "",
            ),
        ];
        let out = render_saved(&records, "program");
        assert!(out.contains("cargo"), "{out}");
        assert!(out.contains("pytest"), "{out}");
        // 5400 bytes saved / 4 = 1350 approximate tokens.
        assert!(out.contains("1350"), "{out}");
        assert!(out.contains("approximate"), "{out}");
        assert!(
            out.contains("not an invoice"),
            "a rule-of-thumb token count must not look precise:\n{out}"
        );
    }

    /// MCP responses are now reported, but in their own labelled block.
    /// The measured section must still say plainly that its own figures
    /// are measured, so the two are never conflated by a skimming reader.
    #[test]
    fn saved_separates_measured_figures_from_the_mcp_estimate() {
        let records = vec![rec(
            repowise_distill::ledger::Kind::Distilled,
            "cargo",
            100,
            10,
            "",
        )];
        let out = render_saved(&records, "program");
        assert!(out.contains("Every figure above is measured"), "{out}");
        // With no MCP records, the block says so rather than vanishing.
        assert!(out.contains("MCP tool responses: none recorded"), "{out}");
    }

    #[test]
    fn saved_can_group_by_day() {
        let records = vec![rec(
            repowise_distill::ledger::Kind::Distilled,
            "cargo",
            400,
            40,
            "",
        )];
        let out = render_saved(&records, "day");
        assert!(out.contains("day "), "{out}");
        assert!(
            !out.contains("cargo"),
            "day grouping shouldn't list programs:\n{out}"
        );
    }

    /// Skipped records only exist if the hook is installed, so an empty
    /// --missed means "not observed", not "nothing missed".
    #[test]
    fn missed_with_no_records_says_nothing_is_being_observed() {
        let out = render_missed(&[]);
        assert!(out.contains("No skipped commands"), "{out}");
        assert!(
            out.contains("not the same as nothing being missed"),
            "{out}"
        );
    }

    /// The two skip reasons mean opposite things for the reader: one is
    /// a gap to close, the other is the design working.
    #[test]
    fn missed_separates_a_widening_candidate_from_a_deliberate_refusal() {
        let records = vec![
            rec(
                repowise_distill::ledger::Kind::Skipped,
                "git",
                0,
                0,
                "not-rewritable",
            ),
            rec(
                repowise_distill::ledger::Kind::Skipped,
                "cargo",
                0,
                0,
                "shell-syntax",
            ),
        ];
        let out = render_missed(&records);
        assert!(out.contains("not-rewritable"), "{out}");
        assert!(out.contains("shell-syntax"), "{out}");
        assert!(out.contains("candidate for widening"), "{out}");
        assert!(
            out.contains("deliberate and will never be"),
            "shell-syntax skips must not read as a coverage gap:\n{out}"
        );
    }

    fn fumble(
        program: &str,
        count: usize,
        codes: Vec<i32>,
    ) -> repowise_distill::corrections::Fumble {
        repowise_distill::corrections::Fumble {
            program: program.to_string(),
            count,
            exit_codes: codes,
        }
    }

    /// Nothing observed and no fumbles are different claims, and the
    /// reader can't tell them apart from a count of findings alone.
    #[test]
    fn corrections_with_nothing_observed_says_so() {
        let out = render_corrections(&[], 0, 2);
        assert!(out.contains("No command runs observed"), "{out}");
        assert!(out.contains("not the same as no fumbles"), "{out}");
    }

    #[test]
    fn corrections_with_observations_but_no_fumbles_reports_the_sample_size() {
        let out = render_corrections(&[], 40, 2);
        assert!(out.contains("40 observed run(s)"), "{out}");
        assert!(
            out.contains("nothing here is inferred"),
            "the report must say the exit codes were observed, not guessed:\n{out}"
        );
    }

    #[test]
    fn corrections_reports_counts_and_distinct_exit_codes() {
        let out = render_corrections(&[fumble("cargo", 3, vec![101])], 50, 2);
        assert!(out.contains("cargo"), "{out}");
        assert!(out.contains("101"), "{out}");
        assert!(out.contains("50 observed run(s)"), "{out}");
        assert!(
            out.contains("several different"),
            "the report should explain what varied codes mean:\n{out}"
        );
    }

    /// Commands carry secrets, and this text gets committed.
    #[test]
    fn the_written_block_carries_program_names_never_argv() {
        let body = corrections_block_body(&[fumble("cargo", 2, vec![101])]);
        assert!(body.contains("`cargo`"), "{body}");
        assert!(
            body.contains("never full command lines"),
            "the block must state its own limit:\n{body}"
        );
        // A flag is `--` immediately followed by a letter; the prose
        // uses `--` as a separator, which is not argv.
        assert!(
            !body.contains("--t") && !body.contains("--w") && !body.contains(" -x"),
            "no flags or argv fragments should appear: {body}"
        );
        // The strongest form of the check: nothing but the program name
        // and counts, so a command that had a token in it can't leak.
        let leaked = corrections_block_body(&[fumble("mytool", 1, vec![1])]);
        assert!(leaked.contains("`mytool`"));
        assert!(!leaked.contains("http"), "{leaked}");
    }

    fn mcp_rec(tool: &str, baseline: usize, response: usize) -> repowise_distill::ledger::Record {
        repowise_distill::ledger::Record {
            at: 1_700_000_000,
            kind: repowise_distill::ledger::Kind::McpResponse,
            program: tool.to_string(),
            raw_bytes: baseline,
            kept_bytes: response,
            detail: String::new(),
            exit_code: None,
        }
    }

    /// The guarantee the whole design rests on: a modelled figure must
    /// never land in a total that claims to be measured.
    #[test]
    fn the_modelled_estimate_is_never_added_to_the_measured_total() {
        let records = vec![
            rec(
                repowise_distill::ledger::Kind::Distilled,
                "cargo",
                4000,
                400,
                "",
            ),
            mcp_rec("get_symbol", 900_000, 500),
        ];
        let out = render_saved(&records, "program");

        // 3600 measured, and the huge modelled number must not appear
        // anywhere in the measured section.
        let measured_section = out.split("MCP tool responses").next().unwrap();
        assert!(measured_section.contains("3600"), "{measured_section}");
        assert!(
            !measured_section.contains("899500") && !measured_section.contains("900000"),
            "the modelled baseline leaked into the measured block:\n{measured_section}"
        );
    }

    #[test]
    fn the_estimate_block_labels_itself_and_states_its_model() {
        let out = render_saved(
            &[
                rec(
                    repowise_distill::ledger::Kind::Distilled,
                    "cargo",
                    100,
                    10,
                    "",
                ),
                mcp_rec("get_context", 8000, 800),
            ],
            "program",
        );
        assert!(out.contains("ESTIMATED, not measured"), "{out}");
        assert!(out.contains("counterfactual"), "{out}");
        assert!(out.contains("NOT added to the measured totals"), "{out}");
        assert!(
            out.contains("upper bound"),
            "the estimate must not read as a floor or an exact figure:\n{out}"
        );
    }

    /// A response bigger than the files it described is a real cost.
    /// `saved_bytes` saturates at zero, so without this the report would
    /// round a loss up to "saved nothing".
    #[test]
    fn a_response_larger_than_its_baseline_is_reported_as_a_cost() {
        let out = render_mcp_estimate(&[mcp_rec("get_context", 500, 9000)]);
        assert!(
            out.contains("returned MORE than the files they described"),
            "{out}"
        );
        assert!(out.contains("8500"), "{out}");
        assert!(out.contains("a real cost, not a saving"), "{out}");
    }

    #[test]
    fn no_cost_warning_when_every_call_saved() {
        let out = render_mcp_estimate(&[mcp_rec("get_symbol", 9000, 500)]);
        assert!(!out.contains("returned MORE"), "{out}");
    }

    /// Nothing recorded means the server hasn't served those tools --
    /// not that they saved nothing.
    #[test]
    fn an_empty_estimate_says_nothing_was_recorded_not_nothing_was_saved() {
        let out = render_mcp_estimate(&[]);
        assert!(out.contains("none recorded"), "{out}");
        assert!(
            out.contains("not that\nthey saved nothing") || out.contains("not that"),
            "{out}"
        );
    }

    /// A ledger holding only MCP records must still show them, rather
    /// than being swallowed by the no-distillations empty state.
    /// `REPOWISE_WORKSPACE` is the plugin's only route to a workspace,
    /// since `.mcp.json` args are static (issue #333).
    ///
    /// These mutate a process-global env var, so they run as one test
    /// rather than several -- `cargo test` runs test fns on parallel
    /// threads, and two tests setting the same variable would race.
    #[test]
    fn workspace_resolution_prefers_the_flag_then_the_env_var() {
        let root = fake_project("workspace-env");
        let ws = root.join("ws.toml");
        std::fs::write(&ws, "[[repo]]\nname = \"a\"\npath = \".\"\n").unwrap();

        // Unset: single-repo, no error.
        unsafe { std::env::remove_var("REPOWISE_WORKSPACE") };
        assert_eq!(resolve_workspace(None).unwrap(), None);

        // The explicit flag wins, and is returned untouched.
        unsafe { std::env::set_var("REPOWISE_WORKSPACE", ws.display().to_string()) };
        let flagged = root.join("other.toml");
        assert_eq!(
            resolve_workspace(Some(flagged.clone())).unwrap(),
            Some(flagged),
            "an explicit --workspace must not be overridden by the environment"
        );

        // No flag: the env var is used.
        assert_eq!(resolve_workspace(None).unwrap(), Some(ws.clone()));

        // Empty/whitespace is treated as unset rather than as a path.
        unsafe { std::env::set_var("REPOWISE_WORKSPACE", "   ") };
        assert_eq!(resolve_workspace(None).unwrap(), None);

        // A path that doesn't exist is a hard error, not a quiet
        // fallback to single-repo: a typo in a globally-exported
        // variable would otherwise be invisible.
        unsafe {
            std::env::set_var(
                "REPOWISE_WORKSPACE",
                root.join("nope.toml").display().to_string(),
            )
        };
        let err = resolve_workspace(None).unwrap_err().to_string();
        assert!(err.contains("does not exist"), "{err}");
        assert!(err.contains("REPOWISE_WORKSPACE"), "{err}");

        unsafe { std::env::remove_var("REPOWISE_WORKSPACE") };
    }

    #[test]
    fn mcp_records_survive_an_empty_distillation_ledger() {
        let out = render_saved(&[mcp_rec("get_symbol", 9000, 500)], "program");
        assert!(out.contains("No distillations recorded"), "{out}");
        assert!(
            out.contains("get_symbol"),
            "an MCP-only ledger must still report its estimate:\n{out}"
        );
    }

    /// Build a throwaway directory for `claude_hook_session_start`
    /// tests -- this crate's own convention (see `hook::tests::
    /// fake_repo`) rather than adding a `tempfile` dev-dependency.
    fn fake_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("repowise-claude-hook-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    /// The sidecar is a *speed* change, never a behaviour change: the
    /// fast path and the fallback must produce the same report (issue
    /// #333). Asserted by running both over one fixture rather than by
    /// hand-writing the expected answer twice.
    #[test]
    fn status_is_identical_with_and_without_the_sidecar() {
        let root = fake_project("sidecar-parity");
        std::fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "pub fn b() {}\n").unwrap();
        let index = repowise_parser::build_index(&root).unwrap();
        index.save(&root).unwrap();

        // Make one file look edited since indexing, so the report has
        // something non-trivial to agree about rather than two empty
        // lists.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(120);
        std::fs::File::options()
            .write(true)
            .open(root.join("a.rs"))
            .unwrap()
            .set_modified(later)
            .unwrap();

        assert!(
            RepoIndex::load_status(&root).is_some(),
            "the fixture must actually have a sidecar, or this proves nothing"
        );
        let fast = collect_status(&root);

        std::fs::remove_file(RepoIndex::status_path(&root)).unwrap();
        assert!(RepoIndex::load_status(&root).is_none(), "fallback path now");
        let slow = collect_status(&root);

        let indexed = fast.indexed.expect("fast path found an index");
        let fallback = slow.indexed.expect("fallback found an index");
        assert_eq!(indexed.file_count, fallback.file_count);
        assert_eq!(indexed.stale, fallback.stale);
        assert_eq!(indexed.missing, fallback.missing);
        assert_eq!(
            indexed.stale.len(),
            1,
            "the fixture must show a stale file, or both paths agreeing on nothing is vacuous"
        );
    }

    #[test]
    fn claude_hook_session_start_bootstraps_an_index_on_first_run() {
        let root = fake_project("bootstrap");
        std::fs::write(root.join("lib.rs"), "pub fn helper() {}\n").unwrap();

        assert!(RepoIndex::load(&root).is_err(), "no index should exist yet");
        let output =
            claude_hook_session_start(&root).expect("a first run always has something to say");
        let text = output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(
            text.contains("indexed 1 file(s) for the first time"),
            "{text}"
        );
        assert_eq!(
            output["hookSpecificOutput"]["hookEventName"],
            "SessionStart"
        );
        assert!(
            RepoIndex::load(&root).is_ok(),
            "the bootstrap must actually persist the index"
        );
    }

    #[test]
    fn claude_hook_session_start_reports_an_up_to_date_index() {
        let root = fake_project("up-to-date");
        std::fs::write(root.join("lib.rs"), "pub fn helper() {}\n").unwrap();
        // Second call, after the first already bootstrapped it.
        claude_hook_session_start(&root);

        let output =
            claude_hook_session_start(&root).expect("an existing index still has status to report");
        let text = output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(text.contains("up to date"), "{text}");
    }

    #[test]
    fn claude_hook_session_start_reports_a_stale_index() {
        let root = fake_project("stale");
        let file = root.join("lib.rs");
        std::fs::write(&file, "pub fn helper() {}\n").unwrap();
        claude_hook_session_start(&root);

        // A real, later write gives the file a strictly newer mtime than
        // the index its own `save` just stamped -- the same mechanism
        // `collect_status`/`repowise status` staleness detection relies
        // on, not a special case for this test.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&file, "pub fn helper() {}\npub fn other() {}\n").unwrap();

        let output =
            claude_hook_session_start(&root).expect("a stale index still has status to report");
        let text = output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(text.contains("stale"), "{text}");
        assert!(text.contains("repowise update"), "{text}");
    }

    /// A root with nothing indexable at all still gets a bootstrap
    /// message -- `build_index` succeeds with zero files, which is a
    /// legitimate (if unusual) first run, not a reason to say nothing.
    #[test]
    fn claude_hook_session_start_bootstraps_even_an_empty_directory() {
        let root = fake_project("empty");

        let output = claude_hook_session_start(&root).expect("even an empty repo bootstraps");
        let text = output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(text.contains("indexed 0 file(s)"), "{text}");
    }

    /// A fixture where `src/lib.rs` imports `src/a.rs`, indexed with
    /// sidecars.
    ///
    /// A real crate layout, not two files at the root: Rust module
    /// resolution maps `mod a;` to a *sibling of the crate root*, so a
    /// flat `a.rs`/`b.rs` pair resolves to nothing and every assertion
    /// below would pass vacuously against an empty dependents map.
    fn project_with_one_dependent(name: &str) -> PathBuf {
        let root = fake_project(name);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"t\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(root.join("src/a.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "mod a;\npub fn go() { a::a() }\n").unwrap();
        let index = repowise_parser::build_index(&root).unwrap();
        save_index_with_sidecars(&index).unwrap();

        // Guard the guard: if resolution ever stops producing this edge,
        // the tests below must fail loudly rather than quietly assert
        // nothing.
        let deps = repowise_graph::load_dependents(&root).expect("fixture must have a sidecar");
        assert_eq!(
            deps.of("src/a.rs"),
            ["src/lib.rs".to_string()],
            "fixture must actually produce a resolved dependent edge"
        );
        root
    }

    fn post_tool_use_input(root: &Path, tool: &str, file: &str) -> String {
        serde_json::json!({
            "tool_name": tool,
            "tool_input": { "file_path": root.join(file).display().to_string() },
        })
        .to_string()
    }

    /// Codex and Claude Code share one hook contract, so one
    /// implementation serves both (issue #333).
    ///
    /// Verified against Codex's published hook docs rather than assumed
    /// from the resemblance: same event names, the same
    /// `hookSpecificOutput` wrapper, `additionalContext` for
    /// `SessionStart`/`PostToolUse`, and
    /// `permissionDecision`/`updatedInput` for `PreToolUse`. Codex's
    /// stdin carries extra fields Claude Code's does not (`turn_id`,
    /// `tool_use_id`), so this feeds a Codex-shaped payload to prove
    /// the extras are ignored rather than tripping the parse.
    #[test]
    fn the_hooks_answer_a_codex_shaped_payload_identically() {
        let root = project_with_one_dependent("codex-shape");

        let pre = serde_json::json!({
            "turn_id": "t1",
            "tool_name": "Bash",
            "tool_use_id": "tu1",
            "tool_input": { "command": "cargo test" },
        })
        .to_string();
        let out = claude_hook_pre_tool_use(&pre).expect("a recognized command is rewritten");
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(out["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .is_some_and(|c| c.starts_with("repowise distill")));

        let post = serde_json::json!({
            "turn_id": "t1",
            "tool_name": "Edit",
            "tool_use_id": "tu2",
            "tool_input": { "file_path": root.join("src/a.rs").display().to_string() },
        })
        .to_string();
        let out = claude_hook_post_tool_use(&root, &post).expect("src/a.rs has a dependent");
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert!(out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .is_some_and(|c| c.contains("import src/a.rs")));
    }

    #[test]
    fn claude_hook_post_tool_use_reports_who_imports_the_edited_file() {
        let root = project_with_one_dependent("post-tool-use");
        let out = claude_hook_post_tool_use(&root, &post_tool_use_input(&root, "Edit", "src/a.rs"))
            .expect("a.rs has a dependent, so there is something to say");
        let text = out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(text.contains("1 file(s) import src/a.rs"), "{text}");
        assert!(text.contains("src/lib.rs"), "{text}");
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    }

    /// Nothing imports `b.rs`, so the hook stays silent rather than
    /// spending context tokens to say "zero".
    #[test]
    fn claude_hook_post_tool_use_says_nothing_for_a_file_with_no_dependents() {
        let root = project_with_one_dependent("post-tool-use-none");
        assert!(claude_hook_post_tool_use(
            &root,
            &post_tool_use_input(&root, "Edit", "src/lib.rs")
        )
        .is_none());
    }

    /// The matcher is the point: `Read`/`Grep` are far more frequent and
    /// the answer isn't actionable then, so matching them would be a
    /// per-call token tax for nothing.
    #[test]
    fn claude_hook_post_tool_use_ignores_tools_other_than_edit_and_write() {
        let root = project_with_one_dependent("post-tool-use-matcher");
        for tool in ["Read", "Grep", "Glob", "Bash"] {
            assert!(
                claude_hook_post_tool_use(&root, &post_tool_use_input(&root, tool, "src/a.rs"))
                    .is_none(),
                "{tool} must not trigger enrichment"
            );
        }
        assert!(
            claude_hook_post_tool_use(&root, &post_tool_use_input(&root, "Write", "src/a.rs"))
                .is_some(),
            "Write must trigger it"
        );
    }

    /// Without a sidecar the hook must say nothing rather than fall back
    /// to loading the index -- that fallback is ~2s in a release build,
    /// paid after every edit.
    #[test]
    fn claude_hook_post_tool_use_is_silent_without_a_sidecar() {
        let root = project_with_one_dependent("post-tool-use-nosidecar");
        std::fs::remove_file(repowise_graph::Dependents::path(&root)).unwrap();
        assert!(
            claude_hook_post_tool_use(&root, &post_tool_use_input(&root, "Edit", "src/a.rs"))
                .is_none()
        );
    }

    #[test]
    fn claude_hook_post_tool_use_fails_open_on_malformed_input() {
        let root = project_with_one_dependent("post-tool-use-malformed");
        assert!(claude_hook_post_tool_use(&root, "not json").is_none());
        assert!(claude_hook_post_tool_use(&root, "{}").is_none());
    }

    #[test]
    fn claude_hook_pre_tool_use_rewrites_a_recognized_command() {
        let input = serde_json::json!({
            "session_id": "abc",
            "tool_name": "Bash",
            "tool_input": { "command": "cargo test" },
        })
        .to_string();

        let output = claude_hook_pre_tool_use(&input).expect("cargo test is rewritable");
        assert_eq!(
            output["hookSpecificOutput"]["updatedInput"]["command"],
            "repowise distill cargo test"
        );
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    }

    #[test]
    fn claude_hook_pre_tool_use_says_nothing_for_an_unrecognized_command() {
        let input = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": "echo hi" },
        })
        .to_string();

        assert!(
            claude_hook_pre_tool_use(&input).is_none(),
            "an unrewritten command has nothing to report"
        );
    }

    #[test]
    fn claude_hook_pre_tool_use_ignores_non_bash_tools() {
        let input = serde_json::json!({
            "tool_name": "Read",
            "tool_input": { "file_path": "/etc/hosts" },
        })
        .to_string();

        assert!(claude_hook_pre_tool_use(&input).is_none());
    }

    #[test]
    fn claude_hook_pre_tool_use_fails_open_on_malformed_input() {
        assert!(claude_hook_pre_tool_use("not json").is_none());
        assert!(claude_hook_pre_tool_use("{}").is_none());
        assert!(
            claude_hook_pre_tool_use(r#"{"tool_name": "Bash"}"#).is_none(),
            "missing tool_input.command"
        );
    }
}
