use crate::Language;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

/// A file found while walking the repo, with its detected language.
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub language: Language,
}

/// Walk `root`, honoring `.gitignore`/`.ignore` files and skipping the
/// repowise index directory itself, returning every regular file found.
pub fn discover_files(root: &Path) -> anyhow::Result<Vec<DiscoveredFile>> {
    let mut out = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != crate::RepoIndex::INDEX_DIR)
        .build();

    for entry in walker {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let language = if is_dockerfile_name(file_name) {
            Language::Dockerfile
        } else {
            path.extension()
                .and_then(|e| e.to_str())
                .map(Language::from_extension)
                .unwrap_or(Language::Other)
        };
        out.push(DiscoveredFile {
            path: path.to_path_buf(),
            language,
        });
    }
    Ok(out)
}

/// Whether `name` follows one of the conventional Dockerfile naming
/// patterns -- checked before extension-based detection, since the most
/// common form (`Dockerfile`) has no extension at all for
/// `Path::extension` to find.
fn is_dockerfile_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "dockerfile" || lower.starts_with("dockerfile.") || lower.ends_with(".dockerfile")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_dockerfile_name_recognizes_the_conventional_forms() {
        assert!(is_dockerfile_name("Dockerfile"));
        assert!(is_dockerfile_name("dockerfile"));
        assert!(is_dockerfile_name("Dockerfile.dev"));
        assert!(is_dockerfile_name("Dockerfile.prod"));
        assert!(is_dockerfile_name("backend.dockerfile"));
    }

    #[test]
    fn is_dockerfile_name_rejects_unrelated_names() {
        assert!(!is_dockerfile_name("main.rs"));
        assert!(!is_dockerfile_name("docker-compose.yml"));
    }
}
