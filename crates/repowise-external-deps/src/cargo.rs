use repowise_core::deps::{DependencyKind, ExternalDependency};
use std::path::Path;

const SECTIONS: &[(&str, DependencyKind)] = &[
    ("dependencies", DependencyKind::Direct),
    ("dev-dependencies", DependencyKind::Dev),
    ("build-dependencies", DependencyKind::Build),
];

pub(crate) fn extract(path: &Path, source: &str) -> Vec<ExternalDependency> {
    let Ok(value) = source.parse::<toml::Value>() else {
        return Vec::new();
    };
    let Some(table) = value.as_table() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (section, kind) in SECTIONS {
        if let Some(deps) = table.get(*section).and_then(|v| v.as_table()) {
            collect_table(deps, *kind, path, source, &mut out);
        }
    }
    // A workspace root's own third-party pins live in
    // `[workspace.dependencies]`, not `[dependencies]` -- most
    // workspace member crates instead write `dep.workspace = true`,
    // which has no version of its own to report (see
    // `dependency_version`'s doc comment).
    if let Some(workspace_deps) = table
        .get("workspace")
        .and_then(|w| w.as_table())
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        collect_table(
            workspace_deps,
            DependencyKind::Direct,
            path,
            source,
            &mut out,
        );
    }
    out
}

fn collect_table(
    deps: &toml::Table,
    kind: DependencyKind,
    path: &Path,
    source: &str,
    out: &mut Vec<ExternalDependency>,
) {
    for (name, spec) in deps {
        if is_workspace_internal(spec) {
            continue;
        }
        out.push(ExternalDependency {
            name: name.clone(),
            version: dependency_version(spec),
            kind,
            ecosystem: "cargo",
            file: path.to_path_buf(),
            line: crate::find_line(source, name),
        });
    }
}

/// A path-only dependency (`{ path = "../sibling" }`, no `version`) is a
/// workspace-internal crate, not a third-party package.
fn is_workspace_internal(spec: &toml::Value) -> bool {
    spec.as_table()
        .is_some_and(|t| t.contains_key("path") && !t.contains_key("version"))
}

/// `None` for `dep.workspace = true` (the version lives in the
/// workspace root's `[workspace.dependencies]`, a separate manifest
/// this function doesn't have access to -- resolving it would be a
/// resolution step, which this crate deliberately doesn't do) and for
/// a git dependency with no `version` key.
fn dependency_version(spec: &toml::Value) -> Option<String> {
    match spec {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Table(t) => t
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_string_version_is_reported() {
        let source = "[dependencies]\nserde = \"1.0\"\n";
        let deps = extract(Path::new("Cargo.toml"), source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde");
        assert_eq!(deps[0].version.as_deref(), Some("1.0"));
        assert_eq!(deps[0].kind, DependencyKind::Direct);
        assert_eq!(deps[0].ecosystem, "cargo");
    }

    #[test]
    fn a_table_with_a_version_key_is_reported() {
        let source = "[dependencies]\ntokio = { version = \"1\", features = [\"full\"] }\n";
        let deps = extract(Path::new("Cargo.toml"), source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "tokio");
        assert_eq!(deps[0].version.as_deref(), Some("1"));
    }

    #[test]
    fn a_path_only_dependency_is_excluded_as_workspace_internal() {
        let source = "[dependencies]\nrepowise-core = { path = \"../repowise-core\" }\n";
        assert!(extract(Path::new("Cargo.toml"), source).is_empty());
    }

    #[test]
    fn a_workspace_inherited_dependency_is_reported_with_no_version() {
        let source = "[dependencies]\nserde = { workspace = true }\n";
        let deps = extract(Path::new("Cargo.toml"), source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde");
        assert_eq!(deps[0].version, None);
    }

    #[test]
    fn dev_and_build_dependencies_get_their_own_kind() {
        let source = "[dev-dependencies]\ntempfile = \"3\"\n\n[build-dependencies]\ncc = \"1\"\n";
        let deps = extract(Path::new("Cargo.toml"), source);
        assert_eq!(deps.len(), 2);
        assert!(deps
            .iter()
            .any(|d| d.name == "tempfile" && d.kind == DependencyKind::Dev));
        assert!(deps
            .iter()
            .any(|d| d.name == "cc" && d.kind == DependencyKind::Build));
    }

    #[test]
    fn workspace_dependencies_are_reported_as_direct() {
        let source = "[workspace.dependencies]\nserde = { version = \"1\" }\n";
        let deps = extract(Path::new("Cargo.toml"), source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde");
        assert_eq!(deps[0].version.as_deref(), Some("1"));
        assert_eq!(deps[0].kind, DependencyKind::Direct);
    }

    #[test]
    fn a_malformed_manifest_is_skipped_not_an_error() {
        assert!(extract(Path::new("Cargo.toml"), "not [ valid toml").is_empty());
    }
}
