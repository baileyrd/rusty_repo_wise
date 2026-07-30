//! Terraform extraction (issue #326, the buildable follow-up to #319's
//! design decision): `TerraformResource`s and `TerraformModule`s from a
//! `.tf` file's top-level `resource`/`module` blocks. A parallel model
//! to `Symbol`/`FileRecord`, computed on demand -- see
//! `repowise_core::terraform`'s module doc for why these are two
//! separate types rather than one shared "declarative object" type.
//!
//! **[`hcl-rs`](https://crates.io/crates/hcl-rs)**, this issue's own
//! original suggestion. It parses generic HCL syntax -- it has no idea
//! `resource`/`module` are Terraform-specific block types, they're just
//! blocks with an identifier and labels like any other (`variable`,
//! `output`, `provider`, `data`, `terraform`, `locals`, ...) -- so
//! `resource`/`module` extraction is a filter over the parsed body, not
//! anything `hcl-rs` itself understands. Those other block types aren't
//! modeled: this issue's own scope is resource/module extraction, not a
//! full Terraform configuration model.
//!
//! Like the other three schema-format crates, `hcl-rs`'s value-oriented
//! parse carries no line/span info, so line numbers fall back to a
//! best-effort text search -- the same tradeoff already made for SQL's
//! `CREATE FUNCTION`/`PROCEDURE`, OpenAPI's schemas/endpoints, and
//! protobuf's messages/services/RPCs.

use hcl::{BlockLabel, Expression, Structure};
use repowise_core::discover_files;
use repowise_core::terraform::{TerraformModule, TerraformResource};
use repowise_core::Language;
use std::path::Path;

/// Walk `root` and extract every `.tf` file's `resource`/`module`
/// blocks. Unreadable files are skipped, matching
/// `repowise_parser::build_index`'s own tolerance for a binary/
/// unreadable file that happened to match. A file that fails to parse
/// as HCL is silently skipped too -- the same graceful-degradation call
/// already made for a malformed `.sql` file.
pub fn collect_terraform(
    root: &Path,
) -> anyhow::Result<(Vec<TerraformResource>, Vec<TerraformModule>)> {
    let root = root.canonicalize()?;
    let discovered = discover_files(&root)?;

    let mut resources = Vec::new();
    let mut modules = Vec::new();
    for entry in discovered {
        if entry.language != Language::Terraform {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&entry.path) else {
            continue;
        };
        let Ok(body) = hcl::parse(&source) else {
            continue;
        };
        let (file_resources, file_modules) = extract_blocks(&entry.path, &source, &body);
        resources.extend(file_resources);
        modules.extend(file_modules);
    }
    Ok((resources, modules))
}

fn extract_blocks(
    path: &Path,
    source: &str,
    body: &hcl::Body,
) -> (Vec<TerraformResource>, Vec<TerraformModule>) {
    let mut resources = Vec::new();
    let mut modules = Vec::new();

    for structure in body.iter() {
        let Structure::Block(block) = structure else {
            continue;
        };
        let labels: Vec<&str> = block.labels.iter().map(label_str).collect();
        match block.identifier.as_str() {
            "resource" if labels.len() >= 2 => {
                let (resource_type, name) = (labels[0].to_string(), labels[1].to_string());
                let line = find_line(source, &[&resource_type, &name]);
                resources.push(TerraformResource {
                    resource_type,
                    name,
                    file: path.to_path_buf(),
                    start_line: line,
                    end_line: line,
                });
            }
            "module" if !labels.is_empty() => {
                let name = labels[0].to_string();
                let source_attr = block_source_attribute(block);
                let line = find_line(source, &[&name]);
                modules.push(TerraformModule {
                    name,
                    source: source_attr,
                    file: path.to_path_buf(),
                    start_line: line,
                    end_line: line,
                });
            }
            // `variable`/`output`/`provider`/`data`/`terraform`/`locals`
            // and any other block type -- out of this issue's scope.
            _ => {}
        }
    }

    (resources, modules)
}

fn label_str(label: &BlockLabel) -> &str {
    match label {
        BlockLabel::Identifier(id) => id.as_str(),
        BlockLabel::String(s) => s.as_str(),
    }
}

fn block_source_attribute(block: &hcl::Block) -> Option<String> {
    block.body.iter().find_map(|s| match s {
        Structure::Attribute(attr) if attr.key.as_str() == "source" => match &attr.expr {
            Expression::String(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    })
}

/// Best-effort line lookup: the first line containing every string in
/// `needles`, e.g. `["aws_instance", "web"]` for
/// `resource "aws_instance" "web" {` -- covers the near-universal
/// single-line block-opener style. Falls back to line 1 if no line
/// matches (an unusually formatted block, or the names appear only as
/// separate tokens across multiple lines).
fn find_line(source: &str, needles: &[&str]) -> usize {
    source
        .lines()
        .enumerate()
        .find(|(_, line)| needles.iter().all(|n| line.contains(n)))
        .map(|(i, _)| i + 1)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TF: &str = r#"
resource "aws_instance" "web" {
  ami           = "ami-123456"
  instance_type = "t2.micro"
}

resource "aws_s3_bucket" "data" {
  bucket = "my-bucket"
}

module "vpc" {
  source = "./modules/vpc"
  cidr   = "10.0.0.0/16"
}

module "no_source" {
  cidr = "10.0.1.0/16"
}

variable "region" {
  default = "us-east-1"
}
"#;

    fn extract(source: &str) -> (Vec<TerraformResource>, Vec<TerraformModule>) {
        let body = hcl::parse(source).expect("valid hcl");
        extract_blocks(Path::new("main.tf"), source, &body)
    }

    #[test]
    fn extracts_resources_with_type_and_name() {
        let (resources, _) = extract(TF);
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].resource_type, "aws_instance");
        assert_eq!(resources[0].name, "web");
        assert_eq!(resources[1].resource_type, "aws_s3_bucket");
        assert_eq!(resources[1].name, "data");
    }

    #[test]
    fn extracts_a_module_with_its_source_attribute() {
        let (_, modules) = extract(TF);
        let vpc = modules
            .iter()
            .find(|m| m.name == "vpc")
            .expect("vpc module");
        assert_eq!(vpc.source.as_deref(), Some("./modules/vpc"));
    }

    #[test]
    fn a_module_without_a_source_attribute_has_none() {
        let (_, modules) = extract(TF);
        let m = modules
            .iter()
            .find(|m| m.name == "no_source")
            .expect("no_source module");
        assert!(m.source.is_none());
    }

    #[test]
    fn variable_and_other_non_resource_module_blocks_are_ignored() {
        let (resources, modules) = extract(TF);
        assert!(resources.iter().all(|r| r.resource_type != "variable"));
        assert!(modules.iter().all(|m| m.name != "region"));
        assert_eq!(resources.len() + modules.len(), 4);
    }

    #[test]
    fn line_numbers_point_at_the_block_opener() {
        let (resources, modules) = extract(TF);
        assert_eq!(resources[0].start_line, 2);
        let vpc = modules.iter().find(|m| m.name == "vpc").unwrap();
        assert_eq!(vpc.start_line, 11);
    }

    #[test]
    fn a_malformed_hcl_file_produces_no_objects_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("bad.tf"), "resource \"aws_instance\" \"web\" {\n").unwrap();

        let (resources, modules) = collect_terraform(&root).unwrap();

        assert!(resources.is_empty());
        assert!(modules.is_empty());
    }

    #[test]
    fn collect_terraform_skips_non_tf_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("notes.txt"), "resource \"x\" \"y\" {}\n").unwrap();

        let (resources, modules) = collect_terraform(&root).unwrap();

        assert!(resources.is_empty());
        assert!(modules.is_empty());
    }
}
