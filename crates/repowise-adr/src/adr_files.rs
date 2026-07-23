use crate::{DecisionRecord, DecisionSource};
use std::path::Path;

/// Parse every `*.md` file under `<root>/docs/adr/`, skipping unfilled
/// template placeholders (a heading whose title is still `<Title>`).
pub fn mine_adr_files(root: &Path) -> anyhow::Result<Vec<DecisionRecord>> {
    let adr_dir = root.join("docs").join("adr");
    if !adr_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(&adr_dir)?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut records = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Some(record) = parse_adr_file(&path)? {
            records.push(record);
        }
    }
    Ok(records)
}

/// Parse a single ADR markdown file matching this repo's template
/// (`# ADR-XXXX: Title`, then `Status:`/`Date:` lines). Returns `None`
/// for an unfilled template (title still `<Title>`) rather than a
/// "decision" with placeholder content.
pub fn parse_adr_file(path: &Path) -> anyhow::Result<Option<DecisionRecord>> {
    let text = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();

    let Some(header_line) = lines.iter().find(|l| l.trim_start().starts_with("# ")) else {
        return Ok(None);
    };
    let header = header_line.trim_start().trim_start_matches("# ").trim();
    let (id, title) = match header.split_once(':') {
        Some((id, title)) => (id.trim().to_string(), title.trim().to_string()),
        None => (header.to_string(), String::new()),
    };
    if title.starts_with('<') && title.ends_with('>') {
        return Ok(None);
    }

    let mut status_raw: Option<String> = None;
    let mut date: Option<String> = None;
    for line in &lines {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Status:") {
            status_raw = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("Date:") {
            date = Some(rest.trim().to_string());
        }
    }

    let superseded_by = status_raw.as_deref().and_then(extract_superseded_by);

    Ok(Some(DecisionRecord {
        id,
        title,
        source: DecisionSource::Adr {
            file: path.to_path_buf(),
        },
        status: status_raw,
        superseded_by,
        date,
        body: text,
        linked_files: Vec::new(),
    }))
}

/// Parse a `Superseded by ADR-XXXX` mention out of a status line
/// (case-insensitive), normalizing to `ADR-XXXX`.
fn extract_superseded_by(status: &str) -> Option<String> {
    let lower = status.to_lowercase();
    let marker_idx = lower.find("superseded by")?;
    let rest = &status[marker_idx..];
    let lower_rest = &lower[marker_idx..];
    let adr_idx = lower_rest.find("adr-")?;
    let digits_start = adr_idx + 4;
    let bytes = rest.as_bytes();
    let mut end = digits_start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == digits_start {
        return None;
    }
    Some(format!("ADR-{}", &rest[digits_start..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_supersession_target() {
        assert_eq!(
            extract_superseded_by("Superseded by ADR-0004"),
            Some("ADR-0004".to_string())
        );
        assert_eq!(extract_superseded_by("Accepted"), None);
    }

    #[test]
    fn skips_unfilled_template() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("0001-template.md");
        std::fs::write(
            &path,
            "# ADR-0001: <Title>\n\nStatus: Proposed | Accepted | Superseded by ADR-XXXX\nDate: YYYY-MM-DD\n",
        )
        .unwrap();
        assert!(parse_adr_file(&path).unwrap().is_none());
    }

    #[test]
    fn parses_a_real_adr() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("0002-use-postgres.md");
        std::fs::write(
            &path,
            "# ADR-0002: Use Postgres for the task queue\n\nStatus: Accepted\nDate: 2026-01-01\n\n## Context\nWe need durable queues.\n",
        )
        .unwrap();
        let record = parse_adr_file(&path).unwrap().unwrap();
        assert_eq!(record.id, "ADR-0002");
        assert_eq!(record.title, "Use Postgres for the task queue");
        assert_eq!(record.status.as_deref(), Some("Accepted"));
        assert_eq!(record.superseded_by, None);
        assert_eq!(record.date.as_deref(), Some("2026-01-01"));
    }
}
