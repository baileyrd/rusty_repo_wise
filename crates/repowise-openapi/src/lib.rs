//! OpenAPI extraction (issue #323, the buildable follow-up to #319's
//! design decision): `OpenApiObject`s from an OpenAPI 3.x document's
//! `paths` (one per HTTP method) and `components.schemas` entries. A
//! parallel model to `Symbol`/`FileRecord`, computed on demand -- see
//! `repowise_core::openapi`'s module doc for why.
//!
//! **No content-sniffing step, and no `Language::OpenApi` variant.**
//! `openapiv3::OpenAPI`'s `openapi`/`info`/`paths` fields are all
//! required (no `Option`, no `#[serde(default)]`) -- a YAML/JSON
//! document that deserializes into it successfully is, by construction,
//! a real OpenAPI 3.x spec. That makes "try to parse, keep it if it
//! works" itself a reliable filter, so every `.yaml`/`.yml`/`.json` file
//! in the repo is a parse *candidate*, and ones that fail (because
//! they're not a spec, or are a Swagger 2.0 document, which this crate
//! doesn't support) are silently skipped rather than reported as
//! errors -- the same graceful-degradation call `repowise_sql` already
//! makes for a malformed non-Jinja `.sql` file.

use openapiv3::{OpenAPI, Operation, PathItem, ReferenceOr, Schema, SchemaKind, Type};
use repowise_core::discover_files;
use repowise_core::openapi::{OpenApiObject, OpenApiObjectKind};
use std::path::Path;

/// Walk `root` and extract every OpenAPI document's schemas and
/// endpoints. Unreadable files are skipped, matching
/// `repowise_parser::build_index`'s own tolerance for a binary/
/// unreadable file that happened to match.
pub fn collect_openapi(root: &Path) -> anyhow::Result<Vec<OpenApiObject>> {
    let root = root.canonicalize()?;
    let discovered = discover_files(&root)?;

    let mut objects = Vec::new();
    for entry in discovered {
        let is_candidate = entry
            .path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| matches!(ext, "yaml" | "yml" | "json"));
        if !is_candidate {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&entry.path) else {
            continue;
        };
        if let Some(spec) = parse_spec(&source) {
            objects.extend(extract_objects(&entry.path, &source, &spec));
        }
    }
    Ok(objects)
}

/// `.json` parses as a strict subset of YAML, but trying JSON first
/// keeps error messages/behavior closer to what a `.json`-specific
/// parser would give for genuinely JSON files; either way a failure
/// here just means "not an OpenAPI document," not an error to surface.
fn parse_spec(source: &str) -> Option<OpenAPI> {
    serde_json::from_str(source)
        .ok()
        .or_else(|| serde_yaml::from_str(source).ok())
}

fn extract_objects(path: &Path, source: &str, spec: &OpenAPI) -> Vec<OpenApiObject> {
    let mut objects = Vec::new();

    for (schema_name, schema_ref) in spec.components.iter().flat_map(|c| c.schemas.iter()) {
        let fields = match schema_ref {
            ReferenceOr::Item(schema) => object_properties(schema),
            ReferenceOr::Reference { .. } => Vec::new(),
        };
        let line = find_line(source, schema_name);
        objects.push(OpenApiObject {
            name: schema_name.clone(),
            kind: OpenApiObjectKind::Schema,
            file: path.to_path_buf(),
            start_line: line,
            end_line: line,
            fields,
        });
    }

    for (path_str, path_item_ref) in spec.paths.iter() {
        let ReferenceOr::Item(path_item) = path_item_ref else {
            continue;
        };
        for (method, operation) in methods(path_item) {
            let name = operation
                .operation_id
                .clone()
                .unwrap_or_else(|| format!("{method} {path_str}"));
            let line = find_line(source, path_str);
            objects.push(OpenApiObject {
                name,
                kind: OpenApiObjectKind::Endpoint,
                file: path.to_path_buf(),
                start_line: line,
                end_line: line,
                fields: Vec::new(),
            });
        }
    }

    objects
}

fn methods(item: &PathItem) -> Vec<(&'static str, &Operation)> {
    let candidates: [(&'static str, &Option<Operation>); 8] = [
        ("GET", &item.get),
        ("PUT", &item.put),
        ("POST", &item.post),
        ("DELETE", &item.delete),
        ("OPTIONS", &item.options),
        ("HEAD", &item.head),
        ("PATCH", &item.patch),
        ("TRACE", &item.trace),
    ];
    candidates
        .into_iter()
        .filter_map(|(method, op)| op.as_ref().map(|op| (method, op)))
        .collect()
}

fn object_properties(schema: &Schema) -> Vec<String> {
    match &schema.schema_kind {
        SchemaKind::Type(Type::Object(obj)) => obj.properties.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// Best-effort line lookup: `openapiv3`'s serde-based deserialization
/// carries no span info at all, so this falls back to a text search for
/// `name` in the raw source -- the same tradeoff already made for SQL's
/// `CREATE FUNCTION`/`PROCEDURE` (see `repowise_sql::extract`).
fn find_line(source: &str, name: &str) -> usize {
    source
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(name))
        .map(|(i, _)| i + 1)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::openapi::OpenApiObjectKind;

    fn extract(source: &str) -> Vec<OpenApiObject> {
        let spec = parse_spec(source).expect("valid spec");
        extract_objects(Path::new("openapi.yaml"), source, &spec)
    }

    const SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Orders API
  version: "1.0"
paths:
  /orders:
    get:
      operationId: listOrders
      responses:
        '200':
          description: ok
  /orders/{id}:
    delete:
      responses:
        '204':
          description: ok
components:
  schemas:
    Order:
      type: object
      properties:
        id:
          type: string
        total:
          type: integer
    OrderList:
      type: array
      items:
        $ref: '#/components/schemas/Order'
"#;

    #[test]
    fn extracts_schemas_with_object_properties() {
        let objects = extract(SPEC);
        let order = objects
            .iter()
            .find(|o| o.kind == OpenApiObjectKind::Schema && o.name == "Order")
            .expect("Order schema");
        let mut fields = order.fields.clone();
        fields.sort();
        assert_eq!(fields, vec!["id", "total"]);
    }

    #[test]
    fn a_non_object_schema_has_no_fields() {
        let objects = extract(SPEC);
        let list = objects
            .iter()
            .find(|o| o.kind == OpenApiObjectKind::Schema && o.name == "OrderList")
            .expect("OrderList schema");
        assert!(list.fields.is_empty());
    }

    #[test]
    fn extracts_one_endpoint_per_method_named_by_operation_id_or_method_and_path() {
        let objects = extract(SPEC);
        let names: Vec<&str> = objects
            .iter()
            .filter(|o| o.kind == OpenApiObjectKind::Endpoint)
            .map(|o| o.name.as_str())
            .collect();
        assert!(names.contains(&"listOrders"));
        assert!(names.contains(&"DELETE /orders/{id}"));
    }

    #[test]
    fn a_document_missing_required_fields_is_not_a_spec() {
        assert!(parse_spec("just: some\nyaml: here\n").is_none());
    }

    #[test]
    fn a_document_missing_required_fields_as_json_is_not_a_spec() {
        assert!(parse_spec(r#"{"hello": "world"}"#).is_none());
    }

    #[test]
    fn collect_openapi_finds_a_real_spec_and_skips_unrelated_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("openapi.yaml"), SPEC).unwrap();
        std::fs::write(root.join("docker-compose.yml"), "services: {}\n").unwrap();

        let objects = collect_openapi(&root).unwrap();

        assert!(objects.iter().any(|o| o.name == "Order"));
        assert!(objects.iter().all(|o| o.file.ends_with("openapi.yaml")));
    }

    #[test]
    fn collect_openapi_reads_json_specs_too() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let json = r#"{
            "openapi": "3.0.0",
            "info": {"title": "t", "version": "1"},
            "paths": {
                "/ping": {"get": {"operationId": "ping", "responses": {}}}
            }
        }"#;
        std::fs::write(root.join("openapi.json"), json).unwrap();

        let objects = collect_openapi(&root).unwrap();

        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].name, "ping");
    }
}
