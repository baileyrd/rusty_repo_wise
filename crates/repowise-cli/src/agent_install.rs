//! `repowise install <host>`: register this repo's MCP server with a
//! coding agent, in that agent's own config format (issue #411).
//!
//! Every supported host consumes the same thing — `repowise serve` over
//! stdio — so the per-host work is a config file, not new capability.
//! What differs is only the schema, and they genuinely do differ:
//! VS Code nests servers under `servers` and wants an explicit
//! `"type": "stdio"`, Cursor and Claude Code use `mcpServers` with no
//! type, opencode uses `mcp` with `"type": "local"` and a single
//! `command` *array*, and Codex uses TOML. Each was checked against
//! that host's own documentation rather than assumed from the others.
//!
//! **Project-level only.** Every host here supports a config file
//! committed inside the repo, and that is the only kind this writes. A
//! user-level install (`~/.codex/config.toml`, `~/.cursor/mcp.json`)
//! would be the first time this tool wrote outside the directory it was
//! pointed at — the git hook is written into `.git/hooks/`, still
//! inside the repo — and that is a decision for issue #411's owner, not
//! a default.
//!
//! This matters mostly for *other* repos. The configs committed at this
//! repo's root only help people working on `rusty_repo_wise` itself;
//! `repowise install` is how someone points a coding agent at their own
//! codebase.

use std::path::{Path, PathBuf};

/// A supported agent host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    ClaudeCode,
    Codex,
    Cursor,
    Opencode,
    VsCode,
}

/// What an install did. Distinguished so the report can say whether
/// anything actually changed, rather than always claiming success.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Created,
    Updated,
    AlreadyCurrent,
    /// The file exists and is not ours to rewrite safely; the caller is
    /// told what to add by hand.
    ManualStepNeeded,
}

impl Host {
    pub const ALL: &'static [Host] = &[
        Host::ClaudeCode,
        Host::Codex,
        Host::Cursor,
        Host::Opencode,
        Host::VsCode,
    ];

    pub fn parse(s: &str) -> Option<Host> {
        match s.to_ascii_lowercase().replace('_', "-").as_str() {
            "claude-code" | "claude" => Some(Host::ClaudeCode),
            "codex" => Some(Host::Codex),
            "cursor" => Some(Host::Cursor),
            "opencode" => Some(Host::Opencode),
            "vscode" | "vs-code" | "copilot" => Some(Host::VsCode),
            _ => None,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Host::ClaudeCode => "claude-code",
            Host::Codex => "codex",
            Host::Cursor => "cursor",
            Host::Opencode => "opencode",
            Host::VsCode => "vscode",
        }
    }

    /// Where this host reads project-scoped MCP config.
    pub fn config_path(self, root: &Path) -> PathBuf {
        match self {
            Host::ClaudeCode => root.join(".mcp.json"),
            Host::Codex => root.join(".codex").join("config.toml"),
            Host::Cursor => root.join(".cursor").join("mcp.json"),
            Host::Opencode => root.join("opencode.json"),
            Host::VsCode => root.join(".vscode").join("mcp.json"),
        }
    }
}

/// The server name every host's config registers us under.
const SERVER_NAME: &str = "repowise";

/// The Codex block, written verbatim when no `.codex/config.toml`
/// exists and printed for the user to paste when one does.
const CODEX_BLOCK: &str = "\n[mcp_servers.repowise]\ncommand = \"repowise\"\nargs = [\"serve\"]\n# Indexing a cold repo can take longer than the 10s default.\nstartup_timeout_sec = 60\n";

/// Register `repowise serve` with `host` for the repo at `root`.
///
/// JSON hosts are **merged**, not overwritten: any other MCP servers
/// already configured are preserved untouched, and only the `repowise`
/// entry is written. Clobbering someone's existing server list to add
/// one entry would be a far worse failure than not installing.
pub fn install(root: &Path, host: Host) -> anyhow::Result<(Outcome, String)> {
    let path = host.config_path(root);
    match host {
        Host::Codex => install_codex(&path),
        _ => install_json(&path, host),
    }
}

fn install_json(path: &Path, host: Host) -> anyhow::Result<(Outcome, String)> {
    // `servers` for VS Code, `mcp` for opencode, `mcpServers` for the
    // rest -- checked per host against its own docs.
    let (key, entry) = match host {
        Host::VsCode => (
            "servers",
            serde_json::json!({ "type": "stdio", "command": "repowise", "args": ["serve"] }),
        ),
        Host::Opencode => (
            "mcp",
            serde_json::json!({ "type": "local", "command": ["repowise", "serve"], "enabled": true }),
        ),
        _ => (
            "mcpServers",
            serde_json::json!({ "command": "repowise", "args": ["serve"] }),
        ),
    };

    let existing = std::fs::read_to_string(path).ok();
    let created = existing.is_none();
    let mut doc: serde_json::Value = match existing.as_deref() {
        None => serde_json::json!({}),
        Some(text) => serde_json::from_str(text).map_err(|e| {
            anyhow::anyhow!(
                "{} exists but is not valid JSON ({e}) -- refusing to overwrite it; \
                 fix or remove it and re-run",
                path.display()
            )
        })?,
    };

    if doc.get(key).and_then(|v| v.get(SERVER_NAME)) == Some(&entry) {
        return Ok((
            Outcome::AlreadyCurrent,
            format!("{} already registers repowise", path.display()),
        ));
    }

    doc.as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("{} is not a JSON object", path.display()))?
        .entry(key)
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("{}'s `{key}` is not an object", path.display()))?
        .insert(SERVER_NAME.to_string(), entry);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", serde_json::to_string_pretty(&doc)?))?;

    Ok(if created {
        (Outcome::Created, format!("created {}", path.display()))
    } else {
        (
            Outcome::Updated,
            format!("updated {} (other servers preserved)", path.display()),
        )
    })
}

/// Codex alone is TOML, and an existing `config.toml` is **never**
/// rewritten.
///
/// A round-trip through a plain TOML parser drops comments and
/// reorders keys, which on a file someone hand-wrote is destructive in
/// a way no install step should be. Detection parses read-only; writing
/// only ever happens when there is no file to damage.
fn install_codex(path: &Path) -> anyhow::Result<(Outcome, String)> {
    let Some(existing) = std::fs::read_to_string(path).ok() else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            path,
            format!(
                "# Codex MCP configuration (written by `repowise install codex`).\n{CODEX_BLOCK}"
            ),
        )?;
        return Ok((Outcome::Created, format!("created {}", path.display())));
    };

    let parsed: toml::Value = toml::from_str(&existing).map_err(|e| {
        anyhow::anyhow!(
            "{} exists but is not valid TOML ({e}) -- refusing to touch it",
            path.display()
        )
    })?;
    if parsed
        .get("mcp_servers")
        .and_then(|v| v.get(SERVER_NAME))
        .is_some()
    {
        return Ok((
            Outcome::AlreadyCurrent,
            format!("{} already registers repowise", path.display()),
        ));
    }

    Ok((
        Outcome::ManualStepNeeded,
        format!(
            "{} already exists and is not rewritten, so its comments and \
             formatting survive. Add:\n{CODEX_BLOCK}",
            path.display()
        ),
    ))
}

/// Per-host report of whether `repowise` is registered.
pub fn status(root: &Path) -> String {
    let mut out = String::from("Agent host MCP registration:\n");
    for host in Host::ALL {
        let path = host.config_path(root);
        let state = if !path.exists() {
            "not configured"
        } else if registers_repowise(&path, *host) {
            "registered"
        } else {
            "config present, repowise not registered"
        };
        out.push_str(&format!(
            "  {:<12} {:<28} {state}\n",
            host.slug(),
            path.strip_prefix(root).unwrap_or(&path).display()
        ));
    }
    out.push_str("\nRun `repowise install <host>` to register one.\n");
    out
}

fn registers_repowise(path: &Path, host: Host) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    match host {
        Host::Codex => toml::from_str::<toml::Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("mcp_servers")
                    .and_then(|m| m.get(SERVER_NAME))
                    .cloned()
            })
            .is_some(),
        _ => {
            let key = match host {
                Host::VsCode => "servers",
                Host::Opencode => "mcp",
                _ => "mcpServers",
            };
            serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get(key).and_then(|m| m.get(SERVER_NAME)).cloned())
                .is_some()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("repowise-install-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    #[test]
    fn every_host_parses_from_its_slug() {
        for host in Host::ALL {
            assert_eq!(Host::parse(host.slug()), Some(*host), "{}", host.slug());
        }
        assert_eq!(
            Host::parse("VSCode"),
            Some(Host::VsCode),
            "case-insensitive"
        );
        assert_eq!(Host::parse("copilot"), Some(Host::VsCode), "alias");
        assert_eq!(Host::parse("nope"), None);
    }

    /// Each host gets its own schema, and they genuinely differ -- this
    /// pins the three shapes so a copy-paste between them can't go
    /// unnoticed.
    #[test]
    fn each_host_writes_the_schema_its_own_docs_specify() {
        let root = scratch("schemas");

        install(&root, Host::VsCode).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(".vscode/mcp.json")).unwrap())
                .unwrap();
        assert_eq!(v["servers"]["repowise"]["type"], "stdio");
        assert_eq!(v["servers"]["repowise"]["command"], "repowise");

        install(&root, Host::Cursor).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(".cursor/mcp.json")).unwrap())
                .unwrap();
        assert_eq!(v["mcpServers"]["repowise"]["command"], "repowise");
        assert!(
            v["mcpServers"]["repowise"].get("type").is_none(),
            "Cursor's schema has no `type` field"
        );

        install(&root, Host::Opencode).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("opencode.json")).unwrap())
                .unwrap();
        assert_eq!(v["mcp"]["repowise"]["type"], "local");
        assert_eq!(
            v["mcp"]["repowise"]["command"],
            serde_json::json!(["repowise", "serve"]),
            "opencode takes one command array, not command + args"
        );
    }

    /// The property that matters most: installing must not destroy
    /// MCP servers someone else configured.
    #[test]
    fn installing_preserves_other_servers_and_unrelated_keys() {
        let root = scratch("merge");
        let path = root.join(".cursor/mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"someOtherKey": 1, "mcpServers": {"theirs": {"command": "their-server"}}}"#,
        )
        .unwrap();

        let (outcome, _) = install(&root, Host::Cursor).unwrap();
        assert_eq!(outcome, Outcome::Updated);

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            v["mcpServers"]["theirs"]["command"], "their-server",
            "another server must survive"
        );
        assert_eq!(
            v["someOtherKey"], 1,
            "unrelated top-level keys must survive"
        );
        assert_eq!(v["mcpServers"]["repowise"]["command"], "repowise");
    }

    #[test]
    fn installing_twice_is_idempotent() {
        let root = scratch("idempotent");
        assert_eq!(install(&root, Host::Cursor).unwrap().0, Outcome::Created);
        assert_eq!(
            install(&root, Host::Cursor).unwrap().0,
            Outcome::AlreadyCurrent
        );
    }

    /// Malformed JSON is refused rather than replaced. Overwriting a
    /// file we could not parse would silently discard whatever the user
    /// meant to have there.
    #[test]
    fn malformed_json_is_refused_not_overwritten() {
        let root = scratch("malformed");
        let path = root.join(".cursor/mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();

        let err = install(&root, Host::Cursor).unwrap_err().to_string();
        assert!(err.contains("not valid JSON"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ not json",
            "the file must be left exactly as it was"
        );
    }

    /// Codex is TOML, and an existing file is never rewritten -- a
    /// parser round-trip would drop the user's comments.
    #[test]
    fn an_existing_codex_config_is_never_rewritten() {
        let root = scratch("codex");
        let path = root.join(".codex/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = "# my careful comment\n[mcp_servers.theirs]\ncommand = \"x\"\n";
        std::fs::write(&path, original).unwrap();

        let (outcome, report) = install(&root, Host::Codex).unwrap();
        assert_eq!(outcome, Outcome::ManualStepNeeded);
        assert!(report.contains("[mcp_servers.repowise]"), "{report}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "the comment and their server must survive untouched"
        );
    }

    #[test]
    fn codex_is_created_when_absent_and_then_detected() {
        let root = scratch("codex-new");
        assert_eq!(install(&root, Host::Codex).unwrap().0, Outcome::Created);
        assert_eq!(
            install(&root, Host::Codex).unwrap().0,
            Outcome::AlreadyCurrent
        );
        assert!(status(&root).contains("registered"));
    }

    #[test]
    fn status_distinguishes_absent_from_present_but_unregistered() {
        let root = scratch("status");
        assert!(status(&root).contains("not configured"));

        let path = root.join(".cursor/mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"mcpServers": {"theirs": {"command": "x"}}}"#).unwrap();
        assert!(status(&root).contains("repowise not registered"));

        install(&root, Host::Cursor).unwrap();
        assert!(status(&root).contains("registered"));
    }
}
