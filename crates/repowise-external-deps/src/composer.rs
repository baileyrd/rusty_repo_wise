use repowise_core::deps::{DependencyKind, ExternalDependency};
use serde_json::Value;
use std::path::Path;

const SECTIONS: &[(&str, DependencyKind)] = &[
    ("require", DependencyKind::Direct),
    ("require-dev", DependencyKind::Dev),
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
            // Composer package names are always `vendor/package`; a key
            // without a `/` is a platform requirement (`php`, `ext-*`,
            // `lib-*`), not a third-party package.
            if !name.contains('/') {
                continue;
            }
            let Some(version) = spec.as_str() else {
                continue;
            };
            out.push(ExternalDependency {
                name: name.clone(),
                version: Some(version.to_string()),
                kind: *kind,
                ecosystem: "composer",
                file: path.to_path_buf(),
                line: crate::find_line(source, name),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_dependency_is_reported() {
        let source = r#"{"require": {"monolog/monolog": "^2.0"}}"#;
        let deps = extract(Path::new("composer.json"), source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "monolog/monolog");
        assert_eq!(deps[0].version.as_deref(), Some("^2.0"));
        assert_eq!(deps[0].ecosystem, "composer");
    }

    #[test]
    fn platform_requirements_without_a_vendor_slash_are_excluded() {
        let source = r#"{"require": {"php": ">=8.1", "ext-json": "*", "monolog/monolog": "^2.0"}}"#;
        let deps = extract(Path::new("composer.json"), source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "monolog/monolog");
    }

    #[test]
    fn require_dev_is_reported_as_dev() {
        let source = r#"{"require-dev": {"phpunit/phpunit": "^9.0"}}"#;
        let deps = extract(Path::new("composer.json"), source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].kind, DependencyKind::Dev);
    }
}
