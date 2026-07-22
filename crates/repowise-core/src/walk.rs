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
        let language = path
            .extension()
            .and_then(|e| e.to_str())
            .map(Language::from_extension)
            .unwrap_or(Language::Other);
        out.push(DiscoveredFile {
            path: path.to_path_buf(),
            language,
        });
    }
    Ok(out)
}
