//! `repowise export` — copy generated wiki pages out to a directory
//! (issue #244).
//!
//! Generated pages otherwise live only under `.repowise/wiki/`, usable
//! in place or through the dashboard. Exporting makes them publishable:
//! a docs site, a PR artifact, an attachment to a review.
//!
//! **Scope.** This module handles the markdown half of `repowise
//! export`. The architecture-model half lives in
//! `repowise_graph::json_graph`; only the non-empty-target guard for it
//! ([`json_graph_dest`]) is here, so both formats share one policy.

use std::path::{Path, PathBuf};

/// One page to copy: its path under the wiki root, and where it lands.
#[derive(Debug, PartialEq, Eq)]
pub struct Planned {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Recursively collect every `.md` file under `dir`, returning paths
/// relative to `dir`.
///
/// Recursive by necessity, not preference: `repowise docs` mirrors the
/// repo's own tree under `.repowise/wiki/`, so a repo whose sources live
/// in subdirectories has *no* pages at the wiki root at all. A
/// non-recursive read would report an empty wiki for almost every real
/// project.
pub fn collect_pages(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(base, &path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

/// Whether `dir` exists and contains anything at all.
fn is_non_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut e| e.next().is_some())
}

/// Plan the copy from `wiki_root` into `out_dir`, preserving the tree.
///
/// Errors rather than producing an empty export when there are no pages:
/// silently writing nothing and reporting success would look identical
/// to a successful export of a repo that genuinely has no docs.
pub fn plan(wiki_root: &Path, out_dir: &Path, force: bool) -> anyhow::Result<Vec<Planned>> {
    if !wiki_root.is_dir() {
        anyhow::bail!(
            "no generated wiki at {} -- run `repowise docs` first",
            wiki_root.display()
        );
    }
    let pages = collect_pages(wiki_root);
    if pages.is_empty() {
        anyhow::bail!(
            "no wiki pages found under {} -- run `repowise docs` first",
            wiki_root.display()
        );
    }
    // Refuse to write into a directory that already has content unless
    // told to. An export target is frequently something like `./docs`,
    // and quietly merging into (or overwriting parts of) a hand-written
    // docs tree would be destructive and hard to undo.
    if !force && is_non_empty_dir(out_dir) {
        anyhow::bail!(
            "{} is not empty -- pass --force to write into it anyway",
            out_dir.display()
        );
    }

    Ok(pages
        .into_iter()
        .map(|rel| Planned {
            from: wiki_root.join(&rel),
            to: out_dir.join(&rel),
        })
        .collect())
}

/// File the JSON-graph export is written to, inside `out_dir`.
pub const JSON_GRAPH_FILE: &str = "architecture.json";

/// Filename for `--format index` (issue #378). Named for what it is —
/// a portable artifact, not the machine-local `.repowise/index.json`.
pub const PORTABLE_INDEX_FILE: &str = "index.portable.json";

/// Prepare `out_dir` for a JSON-graph export and return the file to
/// write.
///
/// Applies the same non-empty guard as the markdown export, with one
/// carve-out: a directory whose only content is a previous
/// `architecture.json` is treated as re-exportable without `--force`.
/// Re-running an export over its own output is the normal case, and
/// demanding `--force` for it would train people to pass `--force`
/// habitually -- which is exactly when it stops protecting anything.
pub fn json_graph_dest(out_dir: &Path, force: bool) -> anyhow::Result<PathBuf> {
    single_file_dest(out_dir, JSON_GRAPH_FILE, force)
}

pub fn portable_index_dest(out_dir: &Path, force: bool) -> anyhow::Result<PathBuf> {
    single_file_dest(out_dir, PORTABLE_INDEX_FILE, force)
}

/// Destination for a one-file export into `out_dir`, refusing to write
/// into a directory holding anything other than that file unless
/// `force`.
fn single_file_dest(out_dir: &Path, name: &str, force: bool) -> anyhow::Result<PathBuf> {
    let dest = out_dir.join(name);
    if !force {
        let other_content = std::fs::read_dir(out_dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.file_name() != name);
        if other_content {
            anyhow::bail!(
                "{} is not empty -- pass --force to write into it anyway",
                out_dir.display()
            );
        }
    }
    std::fs::create_dir_all(out_dir)?;
    Ok(dest)
}

/// Execute a plan, creating parent directories as needed.
pub fn execute(plan: &[Planned]) -> anyhow::Result<usize> {
    for item in plan {
        if let Some(parent) = item.to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&item.from, &item.to)
            .map_err(|e| anyhow::anyhow!("failed to copy {}: {e}", item.from.display()))?;
    }
    Ok(plan.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("repowise-export-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A wiki tree shaped the way `repowise docs` actually writes one:
    /// nested, mirroring the repo, with nothing at the top level.
    fn wiki_with_nested_pages(root: &Path) -> PathBuf {
        let wiki = root.join("wiki");
        std::fs::create_dir_all(wiki.join("crates").join("core").join("src")).unwrap();
        std::fs::create_dir_all(wiki.join("web")).unwrap();
        std::fs::write(
            wiki.join("crates")
                .join("core")
                .join("src")
                .join("lib.rs.md"),
            "# lib.rs\n",
        )
        .unwrap();
        std::fs::write(wiki.join("web").join("app.ts.md"), "# app.ts\n").unwrap();
        wiki
    }

    #[test]
    fn collects_pages_from_nested_directories() {
        // The bug this guards against: a non-recursive scan finds
        // nothing here, because `repowise docs` puts no page at the
        // wiki root for a repo with sources in subdirectories.
        let root = fixture("collect");
        let wiki = wiki_with_nested_pages(&root);
        assert!(
            std::fs::read_dir(&wiki)
                .unwrap()
                .flatten()
                .all(|e| e.path().is_dir()),
            "fixture must have no top-level .md files"
        );

        let pages = collect_pages(&wiki);
        assert_eq!(pages.len(), 2, "{pages:?}");
        assert!(pages.contains(&PathBuf::from("crates/core/src/lib.rs.md")));
        assert!(pages.contains(&PathBuf::from("web/app.ts.md")));
    }

    #[test]
    fn ignores_non_markdown_files() {
        let root = fixture("nonmd");
        let wiki = wiki_with_nested_pages(&root);
        std::fs::write(wiki.join("notes.txt"), "not a page\n").unwrap();
        assert_eq!(collect_pages(&wiki).len(), 2);
    }

    #[test]
    fn exports_preserving_the_tree() {
        let root = fixture("export");
        let wiki = wiki_with_nested_pages(&root);
        let out = root.join("out");

        let plan = plan(&wiki, &out, false).unwrap();
        assert_eq!(execute(&plan).unwrap(), 2);

        assert_eq!(
            std::fs::read_to_string(out.join("crates/core/src/lib.rs.md")).unwrap(),
            "# lib.rs\n"
        );
        assert!(out.join("web/app.ts.md").is_file());
    }

    #[test]
    fn refuses_a_non_empty_target_without_force() {
        let root = fixture("nonempty");
        let wiki = wiki_with_nested_pages(&root);
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("handwritten.md"), "do not clobber me\n").unwrap();

        let err = plan(&wiki, &out, false).unwrap_err().to_string();
        assert!(err.contains("not empty"), "{err}");
        assert!(err.contains("--force"), "{err}");

        // The pre-existing file is untouched.
        assert_eq!(
            std::fs::read_to_string(out.join("handwritten.md")).unwrap(),
            "do not clobber me\n"
        );

        // With --force it proceeds, and still leaves unrelated files alone.
        let plan = plan(&wiki, &out, true).unwrap();
        execute(&plan).unwrap();
        assert!(out.join("web/app.ts.md").is_file());
        assert!(out.join("handwritten.md").is_file());
    }

    #[test]
    fn an_empty_target_directory_does_not_need_force() {
        let root = fixture("emptydir");
        let wiki = wiki_with_nested_pages(&root);
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        assert!(plan(&wiki, &out, false).is_ok());
    }

    #[test]
    fn errors_clearly_when_docs_were_never_generated() {
        let root = fixture("nodocs");
        let err = plan(&root.join("wiki"), &root.join("out"), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("repowise docs"), "{err}");
    }

    #[test]
    fn an_existing_but_empty_wiki_is_an_error_not_a_silent_no_op() {
        // Reporting "exported 0 pages" successfully would be
        // indistinguishable from a real export of a repo with no docs.
        let root = fixture("emptywiki");
        let wiki = root.join("wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let err = plan(&wiki, &root.join("out"), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no wiki pages"), "{err}");
        assert!(err.contains("repowise docs"), "{err}");
    }

    #[test]
    fn json_graph_dest_names_the_file_inside_the_target() {
        let root = fixture("jsondest");
        let out = root.join("out");
        let dest = json_graph_dest(&out, false).unwrap();
        assert_eq!(dest, out.join(JSON_GRAPH_FILE));
        assert!(out.is_dir(), "target directory should be created");
    }

    #[test]
    fn json_graph_refuses_a_target_holding_unrelated_files() {
        let root = fixture("jsonnonempty");
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("notes.md"), "mine\n").unwrap();

        let err = json_graph_dest(&out, false).unwrap_err().to_string();
        assert!(err.contains("not empty"), "{err}");
        assert!(json_graph_dest(&out, true).is_ok(), "--force overrides");
    }

    #[test]
    fn re_exporting_over_a_previous_json_graph_does_not_need_force() {
        // Re-running an export over its own output is the normal case.
        // Demanding --force here would train people to pass it always,
        // which is exactly when it stops protecting anything.
        let root = fixture("jsonreexport");
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join(JSON_GRAPH_FILE), "{}\n").unwrap();
        assert!(json_graph_dest(&out, false).is_ok());
    }
}
