//! SQL/dbt extraction (issue #317, the buildable follow-up to #67's
//! design decision): `SqlObject`s from `CREATE TABLE`/`VIEW`/
//! `FUNCTION`/`PROCEDURE` statements and dbt-model files, plus
//! `LineageEdge`s from `ref()`/`source()` macro calls, resolved against
//! each other where possible. A parallel model to `Symbol`/`FileRecord`,
//! not part of `RepoIndex` -- see `repowise_core::sql`'s module doc for
//! why.

mod extract;

use repowise_core::discover_files;
use repowise_core::sql::{LineageEdge, SqlObject};
use repowise_core::Language;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Walk `root`, extract every `.sql` file's `SqlObject`s and dbt
/// `ref()`/`source()` `LineageEdge`s, then resolve each `ref()` against
/// the other `SqlObject`s discovered in the same repo (matched by name --
/// almost always another dbt model's file-stem-derived name).
/// `source()` calls point at a raw external table declared in a `.yml`
/// config, not a `.sql` file, so they stay unresolved by design -- the
/// same "no index needed" bucket already used for Swift's/Dart's package
/// imports (see `repowise-graph`). Unreadable files are skipped,
/// matching `repowise_parser::build_index`'s own tolerance for a
/// binary/unreadable file that happened to match.
pub fn collect_sql(root: &Path) -> anyhow::Result<(Vec<SqlObject>, Vec<LineageEdge>)> {
    let root = root.canonicalize()?;
    let discovered = discover_files(&root)?;

    let mut objects = Vec::new();
    let mut edges = Vec::new();
    for entry in discovered {
        if entry.language != Language::Sql {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&entry.path) else {
            continue;
        };
        objects.extend(extract::extract_objects(&entry.path, &source));
        edges.extend(extract::extract_lineage(&entry.path, &source));
    }

    let by_name: HashMap<&str, PathBuf> = objects
        .iter()
        .map(|o| (o.name.as_str(), o.file.clone()))
        .collect();
    for edge in &mut edges {
        if let Some(file) = by_name.get(edge.name.as_str()) {
            if *file != edge.from {
                edge.resolved_file = Some(file.clone());
            }
        }
    }

    Ok((objects, edges))
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::sql::{LineageKind, SqlObjectKind};

    #[test]
    fn collect_sql_resolves_a_ref_against_another_discovered_model() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::write(
            root.join("models/stg_orders.sql"),
            "select id from {{ source('raw', 'orders') }}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("models/orders.sql"),
            "select * from {{ ref('stg_orders') }}\n",
        )
        .unwrap();

        let (objects, edges) = collect_sql(&root).unwrap();

        let names: Vec<&str> = {
            let mut v: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();
            v.sort();
            v
        };
        assert_eq!(names, vec!["orders", "stg_orders"]);

        assert_eq!(edges.len(), 2);
        let ref_edge = edges
            .iter()
            .find(|e| e.kind == LineageKind::Ref)
            .expect("a ref() edge from orders.sql");
        assert_eq!(ref_edge.name, "stg_orders");
        assert_eq!(
            ref_edge.resolved_file,
            Some(root.join("models/stg_orders.sql"))
        );
        let source_edge = edges
            .iter()
            .find(|e| e.kind == LineageKind::Source)
            .expect("a source() edge from stg_orders.sql");
        assert_eq!(source_edge.name, "raw.orders");
        assert!(source_edge.resolved_file.is_none());
    }

    #[test]
    fn collect_sql_leaves_a_source_reference_unresolved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("orders.sql"),
            "select * from {{ source('raw', 'orders') }}\n",
        )
        .unwrap();

        let (_, edges) = collect_sql(&root).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, LineageKind::Source);
        assert!(edges[0].resolved_file.is_none());
    }

    #[test]
    fn collect_sql_finds_literal_create_statements_alongside_dbt_models() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("schema.sql"),
            "CREATE TABLE raw_orders (id INT, total INT);\n",
        )
        .unwrap();
        // Not SQL -- must not be picked up.
        std::fs::write(root.join("notes.txt"), "select 1\n").unwrap();

        let (objects, _) = collect_sql(&root).unwrap();

        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].name, "raw_orders");
        assert_eq!(objects[0].kind, SqlObjectKind::Table);
        assert_eq!(objects[0].columns, vec!["id", "total"]);
    }

    #[test]
    fn collect_sql_on_an_empty_repo_finds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (objects, edges) = collect_sql(&root).unwrap();
        assert!(objects.is_empty());
        assert!(edges.is_empty());
    }
}
