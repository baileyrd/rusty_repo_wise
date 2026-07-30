//! Per-file extraction: `SqlObject`s from `CREATE TABLE`/`VIEW`/
//! `FUNCTION`/`PROCEDURE` statements (or, for a dbt model file, the
//! whole file), and dbt `ref()`/`source()` `LineageEdge`s. Resolution
//! against other files happens one level up, in `collect_sql`.

use regex::Regex;
use repowise_core::sql::{LineageEdge, LineageKind, SqlObject, SqlObjectKind};
use sqlparser::ast::{Spanned, Statement};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::path::Path;
use std::sync::OnceLock;

/// dbt models are plain `.sql` files containing Jinja templating this
/// port doesn't compile (`{{ ref(...) }}`, `{{ config(...) }}`, `{% if
/// %}` control blocks) -- `sqlparser` can't parse a file containing
/// those without choking on the literal `{`. Rather than write a
/// Jinja-stripping preprocessor (a project of its own, and out of this
/// issue's scope), a file containing `{{` is treated as one dbt model:
/// the file *is* the object, named after its own stem, per dbt's own
/// "the file's name is the model's name" convention. It's given kind
/// `View` (dbt's default materialization when nothing overrides it --
/// this port doesn't parse `{{ config(materialized=...) }}` to know
/// better, matching #317's "not full dbt semantics" scope).
pub fn extract_objects(path: &Path, source: &str) -> Vec<SqlObject> {
    if source.contains("{{") {
        return vec![dbt_model_object(path, source)];
    }
    match Parser::parse_sql(&GenericDialect {}, source) {
        Ok(statements) => statements
            .iter()
            .filter_map(|stmt| sql_object_from_statement(path, source, stmt))
            .collect(),
        // A dialect-specific extension `GenericDialect` doesn't cover, or
        // a genuinely malformed file -- same graceful-degradation call
        // the Structural tier already makes: no objects rather than
        // failing the whole `collect_sql` walk over one bad file.
        Err(_) => Vec::new(),
    }
}

fn dbt_model_object(path: &Path, source: &str) -> SqlObject {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string();
    SqlObject {
        name,
        kind: SqlObjectKind::View,
        file: path.to_path_buf(),
        start_line: 1,
        end_line: source.lines().count().max(1),
        columns: Vec::new(),
    }
}

fn sql_object_from_statement(path: &Path, source: &str, stmt: &Statement) -> Option<SqlObject> {
    let (name, kind, columns): (String, SqlObjectKind, Vec<String>) = match stmt {
        Statement::CreateTable(t) => (
            t.name.to_string(),
            SqlObjectKind::Table,
            t.columns.iter().map(|c| c.name.to_string()).collect(),
        ),
        Statement::CreateView(v) => (v.name.to_string(), SqlObjectKind::View, Vec::new()),
        Statement::CreateFunction(f) => (f.name.to_string(), SqlObjectKind::Function, Vec::new()),
        Statement::CreateProcedure { name, .. } => {
            (name.to_string(), SqlObjectKind::Procedure, Vec::new())
        }
        _ => return None,
    };
    let (start_line, end_line) = line_range(source, stmt, &name, kind);
    Some(SqlObject {
        name,
        kind,
        file: path.to_path_buf(),
        start_line,
        end_line,
        columns,
    })
}

/// `CreateTable`/`CreateView` carry a real source span via `sqlparser`'s
/// `Spanned` trait; `CreateFunction`/`CreateProcedure` don't yet (its own
/// doc comment lists both as known gaps as of 0.62) and fall back to a
/// text search.
fn line_range(source: &str, stmt: &Statement, name: &str, kind: SqlObjectKind) -> (usize, usize) {
    let span = stmt.span();
    if span.start.line != 0 {
        return (span.start.line as usize, span.end.line as usize);
    }
    let line = find_create_line(source, name, kind).unwrap_or(1);
    (line, line)
}

/// Best-effort fallback for statement kinds `sqlparser` doesn't track
/// spans for: a case-insensitive text search for a line mentioning both
/// the object's keyword (`function`/`procedure`) and its (unqualified)
/// name. Same tradeoff every regex-based extractor in this port already
/// makes -- see `repowise_parser::lightweight`'s module doc for the
/// precedent.
fn find_create_line(source: &str, name: &str, kind: SqlObjectKind) -> Option<usize> {
    let keyword = match kind {
        SqlObjectKind::Function => "function",
        SqlObjectKind::Procedure => "procedure",
        SqlObjectKind::Table | SqlObjectKind::View => return None,
    };
    let short_name = name.rsplit('.').next().unwrap_or(name).to_lowercase();
    source.lines().enumerate().find_map(|(i, line)| {
        let lower = line.to_lowercase();
        (lower.contains("create") && lower.contains(keyword) && lower.contains(&short_name))
            .then_some(i + 1)
    })
}

/// dbt's `{{ ref('model') }}` and `{{ source('src', 'table') }}` macro
/// calls. Only the common single-argument `ref()` form is matched --
/// the two-argument cross-package form (`ref('package', 'model')`) is
/// full dbt-project semantics this issue's own scope excludes.
pub fn extract_lineage(path: &Path, source: &str) -> Vec<LineageEdge> {
    static REF_RE: OnceLock<Regex> = OnceLock::new();
    static SOURCE_RE: OnceLock<Regex> = OnceLock::new();
    let ref_re = REF_RE.get_or_init(|| {
        Regex::new(r#"\{\{\s*ref\(\s*['"]([^'"]+)['"]\s*\)\s*\}\}"#).expect("static regex is valid")
    });
    let source_re = SOURCE_RE.get_or_init(|| {
        Regex::new(r#"\{\{\s*source\(\s*['"]([^'"]+)['"]\s*,\s*['"]([^'"]+)['"]\s*\)\s*\}\}"#)
            .expect("static regex is valid")
    });

    let mut edges = Vec::new();
    for (i, line) in source.lines().enumerate() {
        for cap in ref_re.captures_iter(line) {
            edges.push(LineageEdge {
                from: path.to_path_buf(),
                kind: LineageKind::Ref,
                name: cap[1].to_string(),
                resolved_file: None,
                line: i + 1,
            });
        }
        for cap in source_re.captures_iter(line) {
            edges.push(LineageEdge {
                from: path.to_path_buf(),
                kind: LineageKind::Source,
                name: format!("{}.{}", &cap[1], &cap[2]),
                resolved_file: None,
                line: i + 1,
            });
        }
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    #[test]
    fn create_table_extracts_name_and_columns_with_a_real_span() {
        let source = "CREATE TABLE orders (\n  id INT,\n  total INT\n);\n";
        let objects = extract_objects(&p("orders.sql"), source);
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].name, "orders");
        assert_eq!(objects[0].kind, SqlObjectKind::Table);
        assert_eq!(objects[0].columns, vec!["id", "total"]);
        assert_eq!(objects[0].start_line, 1);
    }

    #[test]
    fn create_view_extracts_name_with_no_columns() {
        let source = "CREATE VIEW active_orders AS SELECT * FROM orders WHERE active;\n";
        let objects = extract_objects(&p("view.sql"), source);
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].name, "active_orders");
        assert_eq!(objects[0].kind, SqlObjectKind::View);
        assert!(objects[0].columns.is_empty());
    }

    #[test]
    fn create_function_falls_back_to_a_text_search_for_its_line() {
        let source = "-- a comment first\nCREATE FUNCTION total_of(order_id INT) RETURNS INT AS $$ SELECT 1 $$ LANGUAGE sql;\n";
        let objects = extract_objects(&p("fn.sql"), source);
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].name, "total_of");
        assert_eq!(objects[0].kind, SqlObjectKind::Function);
        assert_eq!(objects[0].start_line, 2);
    }

    #[test]
    fn multiple_statements_in_one_file_all_extract() {
        let source = "CREATE TABLE a (id INT);\nCREATE TABLE b (id INT);\n";
        let objects = extract_objects(&p("multi.sql"), source);
        let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn a_dbt_model_file_becomes_one_view_named_after_its_file_stem() {
        let source = "select * from {{ ref('stg_orders') }}\n";
        let objects = extract_objects(&p("models/staging/orders.sql"), source);
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].name, "orders");
        assert_eq!(objects[0].kind, SqlObjectKind::View);
        assert!(objects[0].columns.is_empty());
    }

    #[test]
    fn a_malformed_non_jinja_file_produces_no_objects_not_an_error() {
        let source = "CREATE TABLE (((;\n";
        assert!(extract_objects(&p("bad.sql"), source).is_empty());
    }

    #[test]
    fn ref_macro_calls_extract_the_model_name() {
        let source =
            "select *\nfrom {{ ref('stg_orders') }}\njoin {{ ref(\"customers\") }} using (id)\n";
        let edges = extract_lineage(&p("orders.sql"), source);
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].kind, LineageKind::Ref);
        assert_eq!(edges[0].name, "stg_orders");
        assert_eq!(edges[0].line, 2);
        assert_eq!(edges[1].name, "customers");
    }

    #[test]
    fn source_macro_calls_join_the_source_and_table_name() {
        let source = "select * from {{ source('raw', 'orders') }}\n";
        let edges = extract_lineage(&p("orders.sql"), source);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, LineageKind::Source);
        assert_eq!(edges[0].name, "raw.orders");
    }

    #[test]
    fn plain_sql_with_no_jinja_has_no_lineage_edges() {
        let source = "SELECT * FROM orders;\n";
        assert!(extract_lineage(&p("plain.sql"), source).is_empty());
    }
}
