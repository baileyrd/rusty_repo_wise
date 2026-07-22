//! Best-effort mapping from source-level module/import paths to the files
//! that (probably) define them, using directory-layout conventions rather
//! than real compiler resolution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Read a Cargo.toml's `[package] name = "..."` value with a minimal,
/// dependency-free scan (good enough: we only need the first `name = `
/// line under `[package]`).
fn read_package_name(cargo_toml: &Path) -> Option<String> {
    let content = std::fs::read_to_string(cargo_toml).ok()?;
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = trimmed.strip_prefix("name") {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('=') {
                    let value = rest.trim().trim_matches('"');
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Find the nearest ancestor directory containing a `Cargo.toml` and
/// return `(package_name, crate_root_src_dir)` where `crate_root_src_dir`
/// is that directory joined with `src`.
fn find_crate_root(file: &Path) -> Option<(String, PathBuf)> {
    let mut dir = file.parent();
    while let Some(d) = dir {
        let candidate = d.join("Cargo.toml");
        if candidate.is_file() {
            let name = read_package_name(&candidate)?.replace('-', "_");
            return Some((name, d.join("src")));
        }
        dir = d.parent();
    }
    None
}

/// Module path for a Rust source file, e.g. `repowise_core::walk`.
pub fn rust_module_path(file: &Path) -> Option<String> {
    let (crate_name, src_dir) = find_crate_root(file)?;
    let rel = file.strip_prefix(&src_dir).ok()?;
    let mut segments: Vec<String> = rel
        .with_extension("")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if segments.last().map(|s| s.as_str()) == Some("mod") {
        segments.pop();
    }
    if segments.last().map(|s| s.as_str()) == Some("lib")
        || segments.last().map(|s| s.as_str()) == Some("main")
    {
        segments.pop();
    }
    if segments.is_empty() {
        Some(crate_name)
    } else {
        Some(format!("{crate_name}::{}", segments.join("::")))
    }
}

/// Module path for a Python source file relative to the indexed root,
/// e.g. `pkg.sub.foo`.
pub fn python_module_path(file: &Path, root: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).unwrap_or(file);
    let mut segments: Vec<String> = rel
        .with_extension("")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if segments.last().map(|s| s.as_str()) == Some("__init__") {
        segments.pop();
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments.join("."))
    }
}

/// Resolve an import path string against a module index by progressively
/// stripping trailing segments (so importing a specific item from a
/// module still resolves to that module's file).
pub fn resolve_import<'a>(
    import_path: &str,
    sep: &str,
    index: &'a HashMap<String, PathBuf>,
) -> Option<&'a PathBuf> {
    let import_path = import_path.trim_end_matches(&format!("{sep}*"));
    let segments: Vec<&str> = import_path.split(sep).collect();
    for i in (1..=segments.len()).rev() {
        let candidate = segments[..i].join(sep);
        if let Some(p) = index.get(&candidate) {
            return Some(p);
        }
    }
    None
}
