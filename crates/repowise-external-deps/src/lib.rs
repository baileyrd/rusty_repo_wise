//! Third-party dependency extraction (issue #353, the buildable
//! follow-up to the Architecture-restructure issue's Dependencies
//! sub-view). Manifest files are matched by exact filename, the same
//! "no content-sniffing, no `Language` variant" approach
//! `repowise_openapi` already uses (a `Cargo.toml`/`package.json` isn't
//! source code, so it doesn't need `FileRecord`/symbol treatment
//! either) -- see `repowise_core::deps`'s module doc for the
//! declared-not-resolved scope this deliberately stays inside.
//!
//! Five ecosystems, chosen for being cheap to parse correctly without a
//! new dependency: `Cargo.toml` and `pyproject.toml` are TOML (already
//! a workspace dependency via `repowise-workspace`/`repowise-health`),
//! `package.json` and `composer.json` are JSON, `requirements.txt` and
//! `go.mod` are simple line-oriented formats. Java/Kotlin/Scala's
//! `pom.xml`/Gradle build scripts and C#'s `.csproj` need a real XML or
//! Gradle-DSL parser and are deliberately left for a follow-up.

mod cargo;
mod composer;
mod go_mod;
mod npm;
mod pip;

use repowise_core::deps::ExternalDependency;
use repowise_core::discover_files;
use std::path::Path;

/// Walk `root` and extract every recognized manifest's declared
/// third-party dependencies. Unreadable or unparsable manifests are
/// skipped, matching every other schema-format crate's tolerance for a
/// malformed file that happened to match by name.
pub fn collect_dependencies(root: &Path) -> anyhow::Result<Vec<ExternalDependency>> {
    let root = root.canonicalize()?;
    let discovered = discover_files(&root)?;

    let mut out = Vec::new();
    for entry in discovered {
        let file_name = entry
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let Ok(source) = std::fs::read_to_string(&entry.path) else {
            continue;
        };
        let deps = match file_name {
            "Cargo.toml" => cargo::extract(&entry.path, &source),
            "package.json" => npm::extract(&entry.path, &source),
            "composer.json" => composer::extract(&entry.path, &source),
            "requirements.txt" => pip::extract_requirements_txt(&entry.path, &source),
            "pyproject.toml" => pip::extract_pyproject_toml(&entry.path, &source),
            "go.mod" => go_mod::extract(&entry.path, &source),
            _ => continue,
        };
        out.extend(deps);
    }
    Ok(out)
}

/// Best-effort line lookup for a value-oriented (TOML/JSON) parse,
/// which carries no span info -- the same fallback every other
/// schema-format crate in this workspace (`repowise_sql`,
/// `repowise_openapi`, `repowise_protobuf`, `repowise_terraform`) uses.
/// `go_mod` doesn't need this: it parses line-by-line directly, so it
/// always has an exact line number.
pub(crate) fn find_line(source: &str, needle: &str) -> usize {
    source
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(needle))
        .map(|(i, _)| i + 1)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unrelated_repo_reports_no_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

        assert!(collect_dependencies(&root).unwrap().is_empty());
    }

    #[test]
    fn a_repo_with_multiple_manifests_reports_every_ecosystem() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"dependencies": {"react": "^18.0.0"}}"#,
        )
        .unwrap();

        let deps = collect_dependencies(&root).unwrap();
        let ecosystems: std::collections::HashSet<&str> =
            deps.iter().map(|d| d.ecosystem).collect();
        assert!(ecosystems.contains("cargo"), "{deps:?}");
        assert!(ecosystems.contains("npm"), "{deps:?}");
    }
}
