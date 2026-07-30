//! Dockerfile stage extraction (issue #318): no tree-sitter grammar, no
//! external parser crate. Dockerfile syntax is simple and line-oriented
//! enough -- one instruction per logical line, `\` for continuation --
//! that a small hand-written line parser is sufficient; see the issue
//! for why that call was made instead of reaching for a dependency.
//!
//! Deliberately narrow, matching the issue's own prototype scope:
//! parser-directive comments (`# syntax=`, `# escape=`), `ARG`-before-
//! `FROM` variable substitution, and heredoc (`<<EOF`) instruction
//! bodies are not handled.

use repowise_core::docker::{DockerCopyFromEdge, DockerStage};
use std::path::Path;

/// Parse Dockerfile `source` into its stages and any `COPY --from`
/// edges that resolve to another stage in the same file.
pub fn extract_stages(path: &Path, source: &str) -> (Vec<DockerStage>, Vec<DockerCopyFromEdge>) {
    let mut stages: Vec<DockerStage> = Vec::new();
    // (stage index doing the COPY, line, raw --from value)
    let mut copy_froms: Vec<(usize, usize, String)> = Vec::new();

    for (start_line, logical) in logical_lines(source) {
        let trimmed = logical.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let instruction = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();

        if instruction.eq_ignore_ascii_case("FROM") {
            if let Some(prev) = stages.last_mut() {
                prev.end_line = start_line.saturating_sub(1).max(prev.start_line);
            }
            let (base_image, name) = parse_from(rest);
            stages.push(DockerStage {
                file: path.to_path_buf(),
                index: stages.len(),
                name,
                base_image,
                start_line,
                end_line: start_line,
            });
        } else if instruction.eq_ignore_ascii_case("COPY") {
            if let (Some(from_ref), Some(current)) =
                (parse_copy_from(rest), stages.len().checked_sub(1))
            {
                copy_froms.push((current, start_line, from_ref));
            }
        }
    }

    if let Some(last) = stages.last_mut() {
        last.end_line = source.lines().count().max(last.start_line);
    }

    let edges = copy_froms
        .into_iter()
        .filter_map(|(from_stage, line, raw_ref)| {
            resolve_stage_ref(&raw_ref, from_stage, &stages).map(|to_stage| DockerCopyFromEdge {
                file: path.to_path_buf(),
                from_stage,
                to_stage,
                line,
            })
        })
        .collect();

    (stages, edges)
}

/// `FROM [--platform=...] <image> [AS <name>]` -> `(image, name)`.
fn parse_from(rest: &str) -> (String, Option<String>) {
    let tokens: Vec<&str> = rest
        .split_whitespace()
        .filter(|t| !t.starts_with("--"))
        .collect();
    let base_image = tokens.first().map(|s| s.to_string()).unwrap_or_default();
    let name = match tokens.as_slice() {
        [_, as_kw, name, ..] if as_kw.eq_ignore_ascii_case("as") => Some(name.to_string()),
        _ => None,
    };
    (base_image, name)
}

/// The value of a `--from=<X>` flag among `COPY`'s other arguments/
/// flags, if present.
fn parse_copy_from(rest: &str) -> Option<String> {
    rest.split_whitespace()
        .find_map(|tok| tok.strip_prefix("--from=").map(|v| v.to_string()))
}

/// Resolve a `--from` value against stages that appear *before*
/// `current` in the same file — by name, then by numeric index. `None`
/// means it's an external image reference, not a stage in this file.
fn resolve_stage_ref(raw_ref: &str, current: usize, stages: &[DockerStage]) -> Option<usize> {
    let earlier = &stages[..current.min(stages.len())];
    if let Some(stage) = earlier.iter().find(|s| s.name.as_deref() == Some(raw_ref)) {
        return Some(stage.index);
    }
    if let Ok(idx) = raw_ref.parse::<usize>() {
        if idx < earlier.len() {
            return Some(idx);
        }
    }
    None
}

/// Join `\`-continued lines into logical lines, paired with each one's
/// starting 1-based line number. A comment line's trailing `\` does not
/// continue a prior line -- flushed as its own standalone entry instead.
fn logical_lines(source: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut start: Option<usize> = None;

    for (i, raw) in source.lines().enumerate() {
        let line_no = i + 1;
        if raw.trim_start().starts_with('#') {
            if let Some(s) = start.take() {
                out.push((s, std::mem::take(&mut buf)));
            }
            out.push((line_no, raw.to_string()));
            continue;
        }
        if start.is_none() {
            start = Some(line_no);
        }
        match raw.trim_end().strip_suffix('\\') {
            Some(stripped) => {
                buf.push_str(stripped);
                buf.push(' ');
            }
            None => {
                buf.push_str(raw.trim_end());
                out.push((start.take().unwrap(), std::mem::take(&mut buf)));
            }
        }
    }
    if let Some(s) = start {
        out.push((s, buf));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p() -> PathBuf {
        PathBuf::from("Dockerfile")
    }

    #[test]
    fn a_single_unnamed_stage() {
        let source = "FROM rust:1.75\nRUN cargo build --release\n";
        let (stages, edges) = extract_stages(&p(), source);
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].index, 0);
        assert_eq!(stages[0].name, None);
        assert_eq!(stages[0].base_image, "rust:1.75");
        assert_eq!(stages[0].start_line, 1);
        assert_eq!(stages[0].end_line, 2);
        assert!(edges.is_empty());
    }

    #[test]
    fn a_named_multi_stage_build_with_copy_from_by_name() {
        let source = "\
FROM rust:1.75 AS builder
RUN cargo build --release

FROM debian:bookworm-slim AS runtime
COPY --from=builder /app/target/release/app /usr/local/bin/app
CMD [\"/usr/local/bin/app\"]
";
        let (stages, edges) = extract_stages(&p(), source);
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].name.as_deref(), Some("builder"));
        assert_eq!(stages[0].start_line, 1);
        assert_eq!(stages[0].end_line, 3);
        assert_eq!(stages[1].name.as_deref(), Some("runtime"));
        assert_eq!(stages[1].base_image, "debian:bookworm-slim");
        assert_eq!(stages[1].start_line, 4);
        assert_eq!(stages[1].end_line, 6);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_stage, 1);
        assert_eq!(edges[0].to_stage, 0);
        assert_eq!(edges[0].line, 5);
    }

    #[test]
    fn copy_from_by_numeric_index() {
        let source = "FROM golang:1.21\nFROM alpine\nCOPY --from=0 /bin/app /bin/app\n";
        let (stages, edges) = extract_stages(&p(), source);
        assert_eq!(stages.len(), 2);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_stage, 1);
        assert_eq!(edges[0].to_stage, 0);
    }

    /// A `--from` that names an external image, not a stage in this
    /// file, produces no edge -- there's nothing in this repo for it to
    /// point to.
    #[test]
    fn copy_from_an_external_image_produces_no_edge() {
        let source = "FROM alpine\nCOPY --from=alpine:3.18 /etc/ssl /etc/ssl\n";
        let (stages, edges) = extract_stages(&p(), source);
        assert_eq!(stages.len(), 1);
        assert!(edges.is_empty(), "{edges:?}");
    }

    /// A stage can't reference itself or a later stage -- only ones
    /// already declared before it.
    #[test]
    fn copy_from_cannot_reference_the_current_or_a_later_stage() {
        let source = "FROM alpine AS a\nCOPY --from=a /x /x\nFROM alpine AS b\n";
        let (_, edges) = extract_stages(&p(), source);
        assert!(edges.is_empty(), "{edges:?}");
    }

    #[test]
    fn a_platform_flag_before_the_image_does_not_break_parsing() {
        let source = "FROM --platform=linux/amd64 golang:1.21 AS builder\n";
        let (stages, _) = extract_stages(&p(), source);
        assert_eq!(stages[0].base_image, "golang:1.21");
        assert_eq!(stages[0].name.as_deref(), Some("builder"));
    }

    #[test]
    fn line_continuations_are_joined_before_parsing() {
        let source = "FROM alpine\nRUN apt-get update \\\n    && apt-get install -y curl\n";
        let (stages, _) = extract_stages(&p(), source);
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].end_line, 3);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let source = "# syntax=docker/dockerfile:1\n\nFROM alpine\n# a comment\nRUN true\n";
        let (stages, _) = extract_stages(&p(), source);
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].start_line, 3);
    }

    #[test]
    fn instructions_are_case_insensitive() {
        let source = "from alpine as builder\ncopy --from=builder /x /x\n";
        let (stages, _) = extract_stages(&p(), source);
        assert_eq!(stages[0].name.as_deref(), Some("builder"));
    }
}
