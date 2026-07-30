//! GraphQL extraction (issue #325, the buildable follow-up to #319's
//! design decision): `GraphQlObject`s from a GraphQL SDL file's type
//! definitions, plus the fields of its `Query`/`Mutation`/
//! `Subscription` root types. A parallel model to `Symbol`/
//! `FileRecord`, computed on demand -- see `repowise_core::graphql`'s
//! module doc for why.
//!
//! **[`graphql-parser`](https://crates.io/crates/graphql-parser)**,
//! this issue's own original suggestion. Unlike `sqlparser`/
//! `openapiv3`/`protobuf-parse`, every definition and field this crate
//! hands back carries a real parsed `position: Pos` (line/column) --
//! no best-effort text-search fallback needed here, a nicer situation
//! than the other three schema-format crates.
//!
//! **Root-type detection**: `Query`/`Mutation`/`Subscription` aren't a
//! distinct syntax, just ordinary `type` definitions the GraphQL spec
//! treats as schema roots by default name, unless an explicit `schema {
//! query: X, mutation: Y }` block says otherwise. This crate honors
//! that: it looks for a `SchemaDefinition` first, falling back to the
//! default names when none is present -- the common case for real
//! schemas that never bother overriding it.

use graphql_parser::schema::{parse_schema, Definition, Text, TypeDefinition};
use repowise_core::discover_files;
use repowise_core::graphql::{GraphQlObject, GraphQlObjectKind};
use repowise_core::Language;
use std::path::Path;

struct RootNames {
    query: String,
    mutation: String,
    subscription: String,
}

impl Default for RootNames {
    fn default() -> Self {
        RootNames {
            query: "Query".to_string(),
            mutation: "Mutation".to_string(),
            subscription: "Subscription".to_string(),
        }
    }
}

/// Walk `root` and extract every GraphQL SDL file's types, queries,
/// mutations, and subscriptions. Unreadable files are skipped, matching
/// `repowise_parser::build_index`'s own tolerance for a binary/
/// unreadable file that happened to match. A file that fails to parse
/// as GraphQL SDL is silently skipped too -- the same graceful-
/// degradation call already made for a malformed `.sql` file.
pub fn collect_graphql(root: &Path) -> anyhow::Result<Vec<GraphQlObject>> {
    let root = root.canonicalize()?;
    let discovered = discover_files(&root)?;

    let mut objects = Vec::new();
    for entry in discovered {
        if entry.language != Language::GraphQl {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&entry.path) else {
            continue;
        };
        let Ok(doc) = parse_schema::<String>(&source) else {
            continue;
        };
        objects.extend(extract_objects(&entry.path, &doc));
    }
    Ok(objects)
}

fn extract_objects(
    path: &Path,
    doc: &graphql_parser::schema::Document<'_, String>,
) -> Vec<GraphQlObject> {
    let roots = root_names(doc);
    let mut objects = Vec::new();

    for def in &doc.definitions {
        let Definition::TypeDefinition(type_def) = def else {
            continue;
        };
        match type_def {
            TypeDefinition::Object(obj) => {
                let root_kind = if obj.name == roots.query {
                    Some(GraphQlObjectKind::Query)
                } else if obj.name == roots.mutation {
                    Some(GraphQlObjectKind::Mutation)
                } else if obj.name == roots.subscription {
                    Some(GraphQlObjectKind::Subscription)
                } else {
                    None
                };
                match root_kind {
                    Some(kind) => {
                        for field in &obj.fields {
                            let line = field.position.line;
                            objects.push(GraphQlObject {
                                name: format!("{}.{}", obj.name, field.name),
                                kind,
                                file: path.to_path_buf(),
                                start_line: line,
                                end_line: line,
                                fields: Vec::new(),
                            });
                        }
                    }
                    None => {
                        let line = obj.position.line;
                        objects.push(GraphQlObject {
                            name: obj.name.clone(),
                            kind: GraphQlObjectKind::Type,
                            file: path.to_path_buf(),
                            start_line: line,
                            end_line: line,
                            fields: obj.fields.iter().map(|f| f.name.clone()).collect(),
                        });
                    }
                }
            }
            TypeDefinition::Interface(iface) => objects.push(GraphQlObject {
                name: iface.name.clone(),
                kind: GraphQlObjectKind::Type,
                file: path.to_path_buf(),
                start_line: iface.position.line,
                end_line: iface.position.line,
                fields: iface.fields.iter().map(|f| f.name.clone()).collect(),
            }),
            TypeDefinition::Union(u) => objects.push(GraphQlObject {
                name: u.name.clone(),
                kind: GraphQlObjectKind::Type,
                file: path.to_path_buf(),
                start_line: u.position.line,
                end_line: u.position.line,
                fields: u.types.clone(),
            }),
            TypeDefinition::Enum(e) => objects.push(GraphQlObject {
                name: e.name.clone(),
                kind: GraphQlObjectKind::Type,
                file: path.to_path_buf(),
                start_line: e.position.line,
                end_line: e.position.line,
                fields: e.values.iter().map(|v| v.name.clone()).collect(),
            }),
            TypeDefinition::InputObject(io) => objects.push(GraphQlObject {
                name: io.name.clone(),
                kind: GraphQlObjectKind::Type,
                file: path.to_path_buf(),
                start_line: io.position.line,
                end_line: io.position.line,
                fields: io.fields.iter().map(|f| f.name.clone()).collect(),
            }),
            TypeDefinition::Scalar(s) => objects.push(GraphQlObject {
                name: s.name.clone(),
                kind: GraphQlObjectKind::Type,
                file: path.to_path_buf(),
                start_line: s.position.line,
                end_line: s.position.line,
                fields: Vec::new(),
            }),
        }
    }

    objects
}

fn root_names<'a, T: Text<'a>>(doc: &graphql_parser::schema::Document<'a, T>) -> RootNames {
    for def in &doc.definitions {
        if let Definition::SchemaDefinition(sd) = def {
            let defaults = RootNames::default();
            return RootNames {
                query: sd
                    .query
                    .as_ref()
                    .map(|v| v.as_ref().to_string())
                    .unwrap_or(defaults.query),
                mutation: sd
                    .mutation
                    .as_ref()
                    .map(|v| v.as_ref().to_string())
                    .unwrap_or(defaults.mutation),
                subscription: sd
                    .subscription
                    .as_ref()
                    .map(|v| v.as_ref().to_string())
                    .unwrap_or(defaults.subscription),
            };
        }
    }
    RootNames::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::graphql::GraphQlObjectKind;

    const SCHEMA: &str = r#"
schema {
  query: Query
  mutation: Mutation
}

type Query {
  order(id: ID!): Order
  orders: [Order!]!
}

type Mutation {
  createOrder(input: CreateOrderInput!): Order
}

type Order {
  id: ID!
  total: Int!
  status: OrderStatus!
}

input CreateOrderInput {
  customer: String!
  total: Int!
}

enum OrderStatus {
  PENDING
  SHIPPED
}

interface Node {
  id: ID!
}

union SearchResult = Order

scalar DateTime
"#;

    fn extract(source: &str) -> Vec<GraphQlObject> {
        let doc = parse_schema::<String>(source).expect("valid schema");
        extract_objects(Path::new("schema.graphql"), &doc)
    }

    #[test]
    fn extracts_an_object_type_with_field_names() {
        let objects = extract(SCHEMA);
        let order = objects
            .iter()
            .find(|o| o.kind == GraphQlObjectKind::Type && o.name == "Order")
            .expect("Order type");
        assert_eq!(order.fields, vec!["id", "total", "status"]);
    }

    #[test]
    fn root_query_fields_become_flat_query_objects_not_nested_under_query() {
        let objects = extract(SCHEMA);
        assert!(!objects.iter().any(|o| o.name == "Query"));
        let names: Vec<&str> = objects
            .iter()
            .filter(|o| o.kind == GraphQlObjectKind::Query)
            .map(|o| o.name.as_str())
            .collect();
        assert!(names.contains(&"Query.order"));
        assert!(names.contains(&"Query.orders"));
    }

    #[test]
    fn root_mutation_fields_become_flat_mutation_objects() {
        let objects = extract(SCHEMA);
        let mutation = objects
            .iter()
            .find(|o| o.kind == GraphQlObjectKind::Mutation)
            .expect("a Mutation object");
        assert_eq!(mutation.name, "Mutation.createOrder");
    }

    #[test]
    fn extracts_input_enum_interface_union_and_scalar_kinds() {
        let objects = extract(SCHEMA);
        let by_name = |name: &str| objects.iter().find(|o| o.name == name);

        assert_eq!(
            by_name("CreateOrderInput").unwrap().fields,
            vec!["customer", "total"]
        );
        assert_eq!(
            by_name("OrderStatus").unwrap().fields,
            vec!["PENDING", "SHIPPED"]
        );
        assert_eq!(by_name("Node").unwrap().fields, vec!["id"]);
        assert_eq!(by_name("SearchResult").unwrap().fields, vec!["Order"]);
        assert!(by_name("DateTime").unwrap().fields.is_empty());
    }

    #[test]
    fn positions_are_real_not_best_effort() {
        let objects = extract(SCHEMA);
        let order = objects.iter().find(|o| o.name == "Order").unwrap();
        // `type Order {` is the 16th line of `SCHEMA` (1-based, counting
        // the leading blank line from the raw string literal).
        assert_eq!(order.start_line, 16);
    }

    #[test]
    fn without_an_explicit_schema_block_query_and_mutation_default_by_name() {
        let source = "type Query {\n  ping: String\n}\n\ntype Widget {\n  id: ID!\n}\n";
        let objects = extract(source);
        assert!(objects
            .iter()
            .any(|o| o.kind == GraphQlObjectKind::Query && o.name == "Query.ping"));
        assert!(objects
            .iter()
            .any(|o| o.kind == GraphQlObjectKind::Type && o.name == "Widget"));
    }

    #[test]
    fn an_explicit_schema_block_can_rename_the_query_root() {
        let source = "schema {\n  query: RootQuery\n}\n\ntype RootQuery {\n  ping: String\n}\n\ntype Query {\n  legacy: String\n}\n";
        let objects = extract(source);
        assert!(objects
            .iter()
            .any(|o| o.kind == GraphQlObjectKind::Query && o.name == "RootQuery.ping"));
        // `Query` is just an ordinary type name here, not the root.
        assert!(objects
            .iter()
            .any(|o| o.kind == GraphQlObjectKind::Type && o.name == "Query"));
    }

    #[test]
    fn a_malformed_schema_file_produces_no_objects_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("bad.graphql"), "type Order {\n").unwrap();

        let objects = collect_graphql(&root).unwrap();

        assert!(objects.is_empty());
    }

    #[test]
    fn collect_graphql_finds_gql_and_graphql_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.graphql"), "type A {\n  id: ID!\n}\n").unwrap();
        std::fs::write(root.join("b.gql"), "type B {\n  id: ID!\n}\n").unwrap();
        std::fs::write(root.join("notes.txt"), "type C {}\n").unwrap();

        let objects = collect_graphql(&root).unwrap();

        assert!(objects.iter().any(|o| o.name == "A"));
        assert!(objects.iter().any(|o| o.name == "B"));
        assert!(objects.iter().all(|o| o.name != "C"));
    }
}
