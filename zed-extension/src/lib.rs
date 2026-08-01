//! Issue #332's first slice: repowise as a Zed context server (MCP
//! server) extension. Zed's own extension API is narrower than VS
//! Code's -- built around languages, debuggers, themes, snippets, and
//! MCP servers, with no gutter-decoration/hover-provider/CodeLens
//! surface -- so this covers exactly what maps cleanly: registering the
//! existing `repowise serve` MCP server as a Zed context server. No new
//! tools, no new analysis; this is packaging, not new capability, the
//! same shape `claude-plugin/`'s own MCP registration already takes for
//! Claude Code.
//!
//! # Why no explicit project path
//!
//! `context_server_command` receives a `&zed::Project`, whose only
//! public method is `worktree_ids() -> Vec<u64>` -- there is no exposed
//! API to turn a worktree ID into a filesystem path from a context
//! server (unlike `language_server_command`, which receives a
//! `&Worktree` directly, with its own `root_path()`). This relies
//! instead on the same convention every other spawned per-project tool
//! (language servers included) follows: the host sets the subprocess's
//! working directory to the project root. `repowise serve [PATH]`
//! already defaults `PATH` to `.`, so no argument is passed at all.
//! Documented here rather than silently assumed, since it's the one
//! piece of this extension's behavior that isn't directly verifiable
//! against Zed's public extension-API docs.

use zed_extension_api::{self as zed, Command, ContextServerId, Project, Result};

struct RepowiseExtension;

impl zed::Extension for RepowiseExtension {
    fn new() -> Self {
        Self
    }

    fn context_server_command(
        &mut self,
        _context_server_id: &ContextServerId,
        _project: &Project,
    ) -> Result<Command> {
        // Not bundled or downloaded: a compiled Rust binary isn't
        // portable across platforms/architectures the way an extension
        // bundle is, the same reasoning `claude-plugin/`'s own README
        // gives for its MCP registration. Install with
        // `cargo install --path crates/repowise-cli` first.
        Ok(Command {
            command: "repowise".to_string(),
            args: vec!["serve".to_string()],
            env: vec![],
        })
    }
}

zed::register_extension!(RepowiseExtension);
