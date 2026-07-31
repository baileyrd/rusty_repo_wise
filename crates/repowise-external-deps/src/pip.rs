use repowise_core::deps::{DependencyKind, ExternalDependency};
use std::path::Path;

/// Each non-blank, non-comment, non-flag line is a PEP 508 requirement
/// spec (`requests>=2.0`, `black; python_version>='3.8'`, a bare
/// `numpy`). Lines starting with `-` are pip options (`-r other.txt`,
/// `--index-url ...`, `-e ./local-package` for an editable local
/// install) rather than a package, so they're skipped rather than
/// misparsed as one.
pub(crate) fn extract_requirements_txt(path: &Path, source: &str) -> Vec<ExternalDependency> {
    let mut out = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('-') {
            continue;
        }
        if let Some((name, version)) = parse_pep508(line) {
            out.push(ExternalDependency {
                name,
                version,
                kind: DependencyKind::Direct,
                ecosystem: "pypi",
                file: path.to_path_buf(),
                line: i + 1,
            });
        }
    }
    out
}

/// Covers both dependency declaration conventions in use: PEP 621's
/// `[project.dependencies]` (an array of PEP 508 spec strings) and
/// Poetry's `[tool.poetry.dependencies]` (a table mapping name to a
/// version string or a table with a `version` key). A `pyproject.toml`
/// using either (or neither) is handled the same way every other
/// extractor in this crate handles an absent section: nothing to
/// report, not an error.
pub(crate) fn extract_pyproject_toml(path: &Path, source: &str) -> Vec<ExternalDependency> {
    let Ok(value) = source.parse::<toml::Value>() else {
        return Vec::new();
    };
    let Some(table) = value.as_table() else {
        return Vec::new();
    };

    let mut out = Vec::new();

    if let Some(deps) = table
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for spec in deps {
            let Some(spec) = spec.as_str() else { continue };
            if let Some((name, version)) = parse_pep508(spec) {
                out.push(dependency(name, version, path, source));
            }
        }
    }

    if let Some(deps) = table
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for (name, spec) in deps {
            // Poetry always lists the interpreter's own version
            // constraint under the "python" key -- not a package.
            if name.eq_ignore_ascii_case("python") {
                continue;
            }
            let version = match spec {
                toml::Value::String(s) => Some(s.clone()),
                toml::Value::Table(t) => t
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                _ => None,
            };
            out.push(dependency(name.clone(), version, path, source));
        }
    }

    out
}

fn dependency(
    name: String,
    version: Option<String>,
    path: &Path,
    source: &str,
) -> ExternalDependency {
    let line = crate::find_line(source, &name);
    ExternalDependency {
        name,
        version,
        kind: DependencyKind::Direct,
        ecosystem: "pypi",
        file: path.to_path_buf(),
        line,
    }
}

/// Splits a PEP 508 requirement spec into `(name, version_constraint)`:
/// strips an environment marker (after `;`) and extras (`[extra1,extra2]`),
/// then treats whatever's left as the version constraint verbatim
/// (`>=2.0,<3.0`, `==1.0`, `*`) -- this crate reports it as written
/// rather than parsing individual operators, the same "declared, not
/// resolved" scope as every other ecosystem here.
fn parse_pep508(spec: &str) -> Option<(String, Option<String>)> {
    let spec = spec.split(';').next().unwrap_or(spec).trim();
    if spec.is_empty() {
        return None;
    }
    let name_end = spec
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .unwrap_or(spec.len());
    let name = spec[..name_end].trim().to_string();
    if name.is_empty() {
        return None;
    }
    let mut rest = spec[name_end..].trim();
    if let Some(stripped) = rest.strip_prefix('[') {
        rest = match stripped.find(']') {
            Some(end) => stripped[end + 1..].trim(),
            None => "",
        };
    }
    let version = if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    };
    Some((name, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_txt_parses_a_pinned_and_a_bare_package() {
        let source = "requests>=2.0,<3.0\nnumpy\n# a comment\n\n-r other.txt\n";
        let deps = extract_requirements_txt(Path::new("requirements.txt"), source);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "requests");
        assert_eq!(deps[0].version.as_deref(), Some(">=2.0,<3.0"));
        assert_eq!(deps[0].ecosystem, "pypi");
        assert_eq!(deps[1].name, "numpy");
        assert_eq!(deps[1].version, None);
    }

    #[test]
    fn requirements_txt_strips_an_environment_marker_and_extras() {
        let source = "black[jupyter]==23.1.0; python_version>='3.8'\n";
        let deps = extract_requirements_txt(Path::new("requirements.txt"), source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "black");
        assert_eq!(deps[0].version.as_deref(), Some("==23.1.0"));
    }

    #[test]
    fn pep_621_project_dependencies_are_reported() {
        let source = "[project]\ndependencies = [\"requests>=2.0\", \"flask\"]\n";
        let deps = extract_pyproject_toml(Path::new("pyproject.toml"), source);
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d.name == "requests"));
        assert!(deps.iter().any(|d| d.name == "flask"));
    }

    #[test]
    fn poetry_dependencies_exclude_the_python_pseudo_dependency() {
        let source = "[tool.poetry.dependencies]\npython = \"^3.10\"\nrequests = \"^2.0\"\ntoml = { version = \"^0.10\" }\n";
        let deps = extract_pyproject_toml(Path::new("pyproject.toml"), source);
        assert_eq!(deps.len(), 2, "{deps:?}");
        assert!(deps.iter().all(|d| d.name != "python"));
        let toml_dep = deps.iter().find(|d| d.name == "toml").unwrap();
        assert_eq!(toml_dep.version.as_deref(), Some("^0.10"));
    }
}
