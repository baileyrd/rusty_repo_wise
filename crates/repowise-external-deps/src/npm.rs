use repowise_core::deps::{DependencyKind, ExternalDependency};
use serde_json::Value;
use std::path::Path;

const SECTIONS: &[(&str, DependencyKind)] = &[
    ("dependencies", DependencyKind::Direct),
    ("devDependencies", DependencyKind::Dev),
];

pub(crate) fn extract(path: &Path, source: &str) -> Vec<ExternalDependency> {
    let Ok(value) = serde_json::from_str::<Value>(source) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (section, kind) in SECTIONS {
        let Some(deps) = value.get(section).and_then(|v| v.as_object()) else {
            continue;
        };
        for (name, spec) in deps {
            let Some(version) = spec.as_str() else {
                continue;
            };
            if is_local_reference(version) {
                continue;
            }
            out.push(ExternalDependency {
                name: name.clone(),
                version: Some(version.to_string()),
                kind: *kind,
                ecosystem: "npm",
                file: path.to_path_buf(),
                line: crate::find_line(source, name),
            });
        }
    }
    out
}

/// `"file:../sibling"`, `"link:../sibling"`, and workspace-protocol
/// (`"workspace:*"`) references point at a local package in the same
/// repo/monorepo, not a third-party registry package.
fn is_local_reference(version: &str) -> bool {
    version.starts_with("file:")
        || version.starts_with("link:")
        || version.starts_with("workspace:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependencies_and_dev_dependencies_are_reported_with_their_own_kind() {
        let source = r#"{
            "dependencies": {"react": "^18.0.0"},
            "devDependencies": {"jest": "^29.0.0"}
        }"#;
        let deps = extract(Path::new("package.json"), source);
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d.name == "react"
            && d.kind == DependencyKind::Direct
            && d.version.as_deref() == Some("^18.0.0")));
        assert!(deps
            .iter()
            .any(|d| d.name == "jest" && d.kind == DependencyKind::Dev));
    }

    #[test]
    fn a_local_workspace_reference_is_excluded() {
        let source = r#"{"dependencies": {"my-sibling-pkg": "workspace:*"}}"#;
        assert!(extract(Path::new("package.json"), source).is_empty());
    }

    #[test]
    fn malformed_json_is_skipped_not_an_error() {
        assert!(extract(Path::new("package.json"), "{not valid").is_empty());
    }
}
