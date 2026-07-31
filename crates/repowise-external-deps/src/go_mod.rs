use repowise_core::deps::{DependencyKind, ExternalDependency};
use std::path::Path;

/// `go.mod`'s `require` directive, either a single line (`require
/// <path> <version>`) or a parenthesized block (`require (\n <path>
/// <version>\n ... \n)`) -- both are common depending on how many
/// dependencies a module has. Lines marked `// indirect` are a
/// transitive dependency Go recorded for reproducibility, not something
/// this module directly depends on, so they're excluded to keep this
/// "direct dependencies" scope honest.
pub(crate) fn extract(path: &Path, source: &str) -> Vec<ExternalDependency> {
    let mut out = Vec::new();
    let mut in_require_block = false;

    for (i, raw_line) in source.lines().enumerate() {
        let line = strip_line_comment(raw_line).trim();
        let is_indirect = raw_line.contains("// indirect");

        if in_require_block {
            if line == ")" {
                in_require_block = false;
                continue;
            }
            if !line.is_empty() && !is_indirect {
                if let Some((name, version)) = parse_module_version(line) {
                    out.push(dependency(name, version, path, i + 1));
                }
            }
            continue;
        }

        if line == "require (" {
            in_require_block = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("require ") {
            if !is_indirect {
                if let Some((name, version)) = parse_module_version(rest.trim()) {
                    out.push(dependency(name, version, path, i + 1));
                }
            }
        }
    }
    out
}

fn dependency(name: &str, version: &str, path: &Path, line: usize) -> ExternalDependency {
    ExternalDependency {
        name: name.to_string(),
        version: Some(version.to_string()),
        kind: DependencyKind::Direct,
        ecosystem: "go",
        file: path.to_path_buf(),
        line,
    }
}

/// `<module-path> <version>` -- the module path never contains spaces,
/// so splitting on the first run of whitespace is exact, unlike the
/// best-effort text search every value-oriented (TOML/JSON) extractor
/// in this crate falls back to.
fn parse_module_version(s: &str) -> Option<(&str, &str)> {
    let mut parts = s.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim();
    let version = parts.next()?.trim();
    if name.is_empty() || version.is_empty() {
        None
    } else {
        Some((name, version))
    }
}

fn strip_line_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_line_require_is_reported() {
        let source = "module foo\n\ngo 1.21\n\nrequire github.com/foo/bar v1.2.3\n";
        let deps = extract(Path::new("go.mod"), source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "github.com/foo/bar");
        assert_eq!(deps[0].version.as_deref(), Some("v1.2.3"));
        assert_eq!(deps[0].ecosystem, "go");
    }

    #[test]
    fn a_require_block_lists_every_module() {
        let source = "module foo\n\nrequire (\n\tgithub.com/foo/bar v1.2.3\n\tgithub.com/baz/qux v0.1.0\n)\n";
        let deps = extract(Path::new("go.mod"), source);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "github.com/foo/bar");
        assert_eq!(deps[1].name, "github.com/baz/qux");
    }

    #[test]
    fn indirect_dependencies_are_excluded_in_and_out_of_a_block() {
        let source = "require github.com/direct/dep v1.0.0\n\nrequire (\n\tgithub.com/indirect/dep v0.1.0 // indirect\n\tgithub.com/another/direct v2.0.0\n)\n";
        let deps = extract(Path::new("go.mod"), source);
        assert_eq!(deps.len(), 2, "{deps:?}");
        assert!(deps.iter().all(|d| !d.name.contains("indirect")));
    }
}
