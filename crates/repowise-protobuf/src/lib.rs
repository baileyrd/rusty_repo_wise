//! Protobuf extraction (issue #324, the buildable follow-up to #319's
//! design decision): `ProtoObject`s from a `.proto` file's top-level
//! messages, services, and RPCs. A parallel model to `Symbol`/
//! `FileRecord`, computed on demand -- see `repowise_core::protobuf`'s
//! module doc for why.
//!
//! **[`protobuf-parse`](https://crates.io/crates/protobuf-parse)'s pure
//! Rust parser**, not `protoc` -- no external binary dependency. Each
//! file is parsed with the *repo root* as its include path (not the
//! file's own directory), so `import "some/other/dir/thing.proto"`
//! resolves against other `.proto` files anywhere in the repo, matching
//! the common convention of imports being relative to a project root
//! rather than the importing file's own location. `google/protobuf/*`
//! well-known-type imports resolve even without a real file on disk --
//! `protobuf-parse` bundles them internally.
//!
//! A file whose imports don't resolve at all (a genuinely external
//! dependency this repo doesn't vendor) fails to parse, since
//! `parse_and_typecheck` requires every import to resolve -- and is
//! silently skipped, the same graceful-degradation call already made
//! for a malformed `.sql` file or a non-spec YAML/JSON document. This
//! issue's own scope excludes cross-file linking, but not this: a file
//! with an unresolvable import is a real, documented limitation of this
//! first pass, not a deliberate scope cut.

use protobuf::descriptor::FileDescriptorProto;
use protobuf_parse::Parser;
use repowise_core::discover_files;
use repowise_core::protobuf::{ProtoObject, ProtoObjectKind};
use repowise_core::Language;
use std::path::Path;

/// Walk `root` and extract every `.proto` file's messages, services,
/// and RPCs. Unreadable files are skipped, matching
/// `repowise_parser::build_index`'s own tolerance for a binary/
/// unreadable file that happened to match.
pub fn collect_protobuf(root: &Path) -> anyhow::Result<Vec<ProtoObject>> {
    let root = root.canonicalize()?;
    let discovered = discover_files(&root)?;

    let mut objects = Vec::new();
    for entry in discovered {
        if entry.language != Language::Proto {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&entry.path) else {
            continue;
        };
        let Ok(parsed) = Parser::new()
            .pure()
            .include(&root)
            .input(&entry.path)
            .parse_and_typecheck()
        else {
            continue;
        };
        let Some(fd) = find_own_descriptor(&root, &entry.path, &parsed.file_descriptors) else {
            continue;
        };
        objects.extend(extract_objects(&entry.path, &source, fd));
    }
    Ok(objects)
}

/// `parse_and_typecheck` returns one `FileDescriptorProto` per resolved
/// file, including every transitive import -- this picks out the one
/// for `path` itself, so a message imported *into* several files isn't
/// reported once per importer.
fn find_own_descriptor<'a>(
    root: &Path,
    path: &Path,
    descriptors: &'a [FileDescriptorProto],
) -> Option<&'a FileDescriptorProto> {
    let relative = path
        .strip_prefix(root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    descriptors.iter().find(|fd| fd.name() == relative)
}

fn extract_objects(path: &Path, source: &str, fd: &FileDescriptorProto) -> Vec<ProtoObject> {
    let mut objects = Vec::new();

    for message in &fd.message_type {
        let name = message.name().to_string();
        let fields = message.field.iter().map(|f| f.name().to_string()).collect();
        let line = find_line(source, &name);
        objects.push(ProtoObject {
            name,
            kind: ProtoObjectKind::Message,
            file: path.to_path_buf(),
            start_line: line,
            end_line: line,
            fields,
        });
    }

    for service in &fd.service {
        let service_name = service.name().to_string();
        let line = find_line(source, &service_name);
        objects.push(ProtoObject {
            name: service_name.clone(),
            kind: ProtoObjectKind::Service,
            file: path.to_path_buf(),
            start_line: line,
            end_line: line,
            fields: Vec::new(),
        });

        for method in &service.method {
            let name = format!("{service_name}.{}", method.name());
            let line = find_line(source, method.name());
            objects.push(ProtoObject {
                name,
                kind: ProtoObjectKind::Rpc,
                file: path.to_path_buf(),
                start_line: line,
                end_line: line,
                fields: vec![
                    unqualify(method.input_type()),
                    unqualify(method.output_type()),
                ],
            });
        }
    }

    objects
}

/// `MethodDescriptorProto.input_type`/`.output_type` are the *fully
/// qualified* type name, with a leading `.` marking "from the root
/// package" (e.g. `.orders.GetOrderRequest`) -- protobuf's own
/// descriptor convention, not how anyone writes it in `.proto` source.
/// Strips that leading dot for display.
fn unqualify(type_name: &str) -> String {
    type_name.strip_prefix('.').unwrap_or(type_name).to_string()
}

/// Best-effort line lookup: the descriptor types this crate hands back
/// carry no span info at all, so this falls back to a text search for
/// `name` in the raw source -- the same tradeoff already made for SQL's
/// `CREATE FUNCTION`/`PROCEDURE` and OpenAPI's schemas/endpoints.
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
    use repowise_core::protobuf::ProtoObjectKind;

    #[test]
    fn extracts_a_message_with_its_field_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("order.proto"),
            "syntax = \"proto3\";\n\nmessage Order {\n  string id = 1;\n  int32 total = 2;\n}\n",
        )
        .unwrap();

        let objects = collect_protobuf(&root).unwrap();

        let order = objects
            .iter()
            .find(|o| o.kind == ProtoObjectKind::Message && o.name == "Order")
            .expect("Order message");
        assert_eq!(order.fields, vec!["id", "total"]);
    }

    #[test]
    fn extracts_a_service_and_its_rpcs_as_separate_flat_objects() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("order.proto"),
            "syntax = \"proto3\";\n\n\
             message GetOrderRequest { string id = 1; }\n\
             message Order { string id = 1; }\n\n\
             service OrderService {\n  rpc GetOrder(GetOrderRequest) returns (Order);\n}\n",
        )
        .unwrap();

        let objects = collect_protobuf(&root).unwrap();

        assert!(objects
            .iter()
            .any(|o| o.kind == ProtoObjectKind::Service && o.name == "OrderService"));
        let rpc = objects
            .iter()
            .find(|o| o.kind == ProtoObjectKind::Rpc)
            .expect("a Rpc object");
        assert_eq!(rpc.name, "OrderService.GetOrder");
        assert_eq!(rpc.fields, vec!["GetOrderRequest", "Order"]);
    }

    #[test]
    fn resolves_a_well_known_type_import_without_a_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("event.proto"),
            "syntax = \"proto3\";\n\
             import \"google/protobuf/timestamp.proto\";\n\n\
             message Event {\n  google.protobuf.Timestamp created_at = 1;\n}\n",
        )
        .unwrap();

        let objects = collect_protobuf(&root).unwrap();

        assert!(objects.iter().any(|o| o.name == "Event"));
    }

    #[test]
    fn resolves_a_cross_directory_import_within_the_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("common")).unwrap();
        std::fs::create_dir_all(root.join("orders")).unwrap();
        std::fs::write(
            root.join("common/types.proto"),
            "syntax = \"proto3\";\npackage common;\nmessage Money {\n  int64 cents = 1;\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("orders/order.proto"),
            "syntax = \"proto3\";\npackage orders;\nimport \"common/types.proto\";\n\n\
             message Order {\n  common.Money total = 1;\n}\n",
        )
        .unwrap();

        let objects = collect_protobuf(&root).unwrap();

        // Both files' own messages are reported -- `Money` once (from
        // common/types.proto's own descriptor), not once per importer.
        let money_count = objects.iter().filter(|o| o.name == "Money").count();
        assert_eq!(money_count, 1);
        assert!(objects.iter().any(|o| o.name == "Order"));
    }

    #[test]
    fn a_proto_file_with_an_unresolvable_import_is_silently_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("event.proto"),
            "syntax = \"proto3\";\nimport \"some/vendored/package.proto\";\n\n\
             message Event {\n  string id = 1;\n}\n",
        )
        .unwrap();

        let objects = collect_protobuf(&root).unwrap();

        assert!(objects.is_empty());
    }

    #[test]
    fn collect_protobuf_skips_non_proto_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("notes.txt"), "message Order {}\n").unwrap();

        let objects = collect_protobuf(&root).unwrap();

        assert!(objects.is_empty());
    }
}
