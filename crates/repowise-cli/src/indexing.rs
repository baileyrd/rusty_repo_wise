use repowise_core::{discover_files, FileRecord, Language, RepoIndex};
use std::path::Path;

/// Walk `root`, parse every Rust/Python file, and return the resulting
/// index (not yet saved to disk).
pub fn build_index(root: &Path) -> anyhow::Result<RepoIndex> {
    let root = root.canonicalize()?;
    let discovered = discover_files(&root)?;

    let mut files: Vec<FileRecord> = Vec::new();
    let mut other_files = 0usize;

    for entry in discovered {
        if matches!(entry.language, Language::Other) {
            other_files += 1;
            continue;
        }
        let source = match std::fs::read_to_string(&entry.path) {
            Ok(s) => s,
            Err(_) => {
                // Binary or unreadable file that happened to match an
                // extension; count it and move on.
                other_files += 1;
                continue;
            }
        };
        match repowise_parser::parse_file(&entry.path, entry.language, &source)? {
            Some(record) => files.push(record),
            None => other_files += 1,
        }
    }

    Ok(RepoIndex {
        root,
        files,
        other_files,
    })
}
