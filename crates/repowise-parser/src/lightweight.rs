//! The "Lightweight" tier (issue #69): Elixir, Clojure, Haskell, Lean 4,
//! Erlang, and F# each get a file-level import graph via regex, and
//! nothing else — no symbols, no calls, no complexity. A deliberately
//! shallower extraction pipeline than every other supported language,
//! which all get full tree-sitter AST extraction.
//!
//! **Imports are left unresolved.** Each language's module-naming
//! convention diverges from the simple "dotted segment = directory
//! name" mapping this crate already has for Python/Java/Go/etc. in
//! non-trivial, language-specific ways (Elixir's `MyApp.Foo` ->
//! `my_app/foo.ex` needs a CamelCase-to-snake_case conversion; Clojure's
//! `my-app.core` -> `my_app/core.clj` needs hyphen-to-underscore;
//! Erlang's module references are flat, not dotted; F#'s `open` doesn't
//! reliably correspond to a file path at all). Building six
//! language-specific resolvers is a bigger, separately-decidable
//! commitment than "extract what a file imports" — so every `ImportRef`
//! here carries `resolved_file: None` by design, the same choice already
//! made for Swift's and Dart's package imports (see
//! `repowise-graph`'s own doc comment on that bucket).
//!
//! **A regex, not a parser.** Every match here is a single-line pattern
//! (Clojure's dotted-token match is the only one applied per-line rather
//! than requiring a specific keyword prefix, since its `:require`/
//! `:import` forms are s-expressions this crate makes no attempt to
//! actually parse — see that function's own doc comment). False
//! positives are accepted, the same tradeoff `repowise-adr`'s
//! keyword-heuristic decision sources already make.

use repowise_core::{FileRecord, ImportRef, Language};
use std::path::Path;
use std::sync::OnceLock;

/// Extract a bare `FileRecord` for one of the six Lightweight-tier
/// languages: real `imports`, empty everything else.
pub fn extract(path: &Path, language: Language, source: &str) -> FileRecord {
    let imports = match language {
        Language::Elixir => extract_elixir_imports(source),
        Language::Clojure => extract_clojure_imports(source),
        Language::Haskell => extract_haskell_imports(source),
        Language::Lean => extract_lean_imports(source),
        Language::Erlang => extract_erlang_imports(source),
        Language::FSharp => extract_fsharp_imports(source),
        _ => unreachable!("extract is only called for Lightweight-tier languages"),
    };
    FileRecord {
        path: path.to_path_buf(),
        language,
        lines: source.lines().count(),
        symbols: Vec::new(),
        imports,
        calls: Vec::new(),
        field_accesses: Vec::new(),
    }
}

fn import_ref(path: &str, line: usize) -> ImportRef {
    ImportRef {
        path: path.to_string(),
        line,
        resolved_file: None,
    }
}

/// One capture-group regex applied to each line, producing one
/// `ImportRef` per match.
fn extract_by_line_regex(source: &str, regex: &regex::Regex) -> Vec<ImportRef> {
    source
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            regex
                .captures(line)
                .and_then(|c| c.get(1))
                .map(|m| import_ref(m.as_str(), i + 1))
        })
        .collect()
}

/// `import Foo.Bar`, `alias Foo.Bar`, `alias Foo.Bar, as: Baz`,
/// `require Foo.Bar`, `use Foo.Bar`.
fn extract_elixir_imports(source: &str) -> Vec<ImportRef> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^\s*(?:import|alias|require|use)\s+([A-Z][\w.]*)")
            .expect("static regex is valid")
    });
    extract_by_line_regex(source, re)
}

/// `import Data.Text.Internal`, `import qualified Data.Text as T`.
fn extract_haskell_imports(source: &str) -> Vec<ImportRef> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^\s*import\s+(?:qualified\s+)?([A-Z][\w.]*)")
            .expect("static regex is valid")
    });
    extract_by_line_regex(source, re)
}

/// `import Mathlib.Algebra.Group`.
fn extract_lean_imports(source: &str) -> Vec<ImportRef> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^\s*import\s+([A-Za-z][\w.]*)").expect("static regex is valid")
    });
    extract_by_line_regex(source, re)
}

/// `open Namespace.Module`.
fn extract_fsharp_imports(source: &str) -> Vec<ImportRef> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^\s*open\s+([A-Z][\w.]*)").expect("static regex is valid")
    });
    extract_by_line_regex(source, re)
}

/// `-import(module, [...]).` (a flat module atom, not dotted -- Erlang
/// has no namespacing) and `-include("path.hrl")`/
/// `-include_lib("app/include/path.hrl")` (a literal include path, the
/// closest thing this language has to a second import form).
fn extract_erlang_imports(source: &str) -> Vec<ImportRef> {
    static IMPORT_RE: OnceLock<regex::Regex> = OnceLock::new();
    static INCLUDE_RE: OnceLock<regex::Regex> = OnceLock::new();
    let import_re = IMPORT_RE.get_or_init(|| {
        regex::Regex::new(r#"-import\(\s*([a-zA-Z_][\w]*)\s*,"#).expect("static regex is valid")
    });
    let include_re = INCLUDE_RE.get_or_init(|| {
        regex::Regex::new(r#"-include(?:_lib)?\(\s*"([^"]+)"\s*\)"#).expect("static regex is valid")
    });

    source
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            import_re
                .captures(line)
                .or_else(|| include_re.captures(line))
                .and_then(|c| c.get(1))
                .map(|m| import_ref(m.as_str(), i + 1))
        })
        .collect()
}

/// Clojure's `(:require [foo.bar :as fb])`/`(:import [java.util Date])`
/// forms are s-expressions this crate makes no attempt to parse -- there
/// is no reliable single-line regex for "inside a `:require`/`:import`
/// form" without tracking paren depth. Instead: any bracketed or
/// quoted **dotted** token (2+ segments, e.g. `foo.bar` or
/// `clojure.string`) is treated as a namespace reference, matching the
/// overwhelmingly common style-guide convention of one namespace per
/// line. A bare local-binding vector like `[x 1]` has no dot in it and
/// doesn't match; a data literal that happens to contain a dotted,
/// bracketed token is the accepted false-positive case for this tier.
fn extract_clojure_imports(source: &str) -> Vec<ImportRef> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"[\['] *([a-zA-Z][\w-]*(?:\.[a-zA-Z][\w-]*)+)")
            .expect("static regex is valid")
    });
    source
        .lines()
        .enumerate()
        .flat_map(|(i, line)| {
            re.captures_iter(line)
                .map(move |c| import_ref(c.get(1).unwrap().as_str(), i + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elixir_extracts_import_alias_require_and_use() {
        let source = "import Foo.Bar\nalias Foo.Baz, as: Baz\nrequire Logger\nuse Ecto.Schema\n";
        let imports = extract_elixir_imports(source);
        let paths: Vec<&str> = imports.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, vec!["Foo.Bar", "Foo.Baz", "Logger", "Ecto.Schema"]);
        assert_eq!(imports[0].line, 1);
        assert!(imports.iter().all(|i| i.resolved_file.is_none()));
    }

    #[test]
    fn haskell_extracts_plain_and_qualified_imports() {
        let source = "import Data.Text\nimport qualified Data.Text as T\n";
        let imports = extract_haskell_imports(source);
        let paths: Vec<&str> = imports.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, vec!["Data.Text", "Data.Text"]);
        assert_eq!(imports[1].line, 2);
    }

    #[test]
    fn lean_extracts_import_lines() {
        let source = "import Mathlib.Algebra.Group\nimport Mathlib.Data.Nat.Basic\n";
        let imports = extract_lean_imports(source);
        let paths: Vec<&str> = imports.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["Mathlib.Algebra.Group", "Mathlib.Data.Nat.Basic"]
        );
    }

    #[test]
    fn fsharp_extracts_open_statements() {
        let source = "module MyApp.Program\n\nopen System\nopen MyApp.Utils\n";
        let imports = extract_fsharp_imports(source);
        let paths: Vec<&str> = imports.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, vec!["System", "MyApp.Utils"]);
        assert_eq!(imports[0].line, 3);
    }

    #[test]
    fn erlang_extracts_import_and_include_forms() {
        let source = "-module(my_mod).\n-import(lists, [map/2]).\n\
             -include(\"records.hrl\").\n-include_lib(\"kernel/include/file.hrl\").\n";
        let imports = extract_erlang_imports(source);
        let paths: Vec<&str> = imports.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["lists", "records.hrl", "kernel/include/file.hrl"]
        );
    }

    #[test]
    fn clojure_extracts_one_namespace_per_bracketed_line() {
        let source = "(ns my-app.core\n  (:require [clojure.string :as str]\n            [my-app.util :refer [helper]])\n  (:import [java.util Date]))\n";
        let imports = extract_clojure_imports(source);
        let paths: Vec<&str> = imports.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, vec!["clojure.string", "my-app.util", "java.util"]);
    }

    #[test]
    fn clojure_bare_quoted_require_is_also_matched() {
        let source = "(require 'clojure.set)\n(import 'java.util.Date)\n";
        let imports = extract_clojure_imports(source);
        let paths: Vec<&str> = imports.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, vec!["clojure.set", "java.util.Date"]);
    }

    #[test]
    fn clojure_local_bindings_without_a_dot_are_not_matched() {
        let source = "(let [x 1\n      y 2]\n  (+ x y))\n";
        assert!(extract_clojure_imports(source).is_empty());
    }

    #[test]
    fn extract_returns_a_bare_record_with_only_imports_populated() {
        let path = Path::new("lib/my_app.ex");
        let source = "import Foo.Bar\n";
        let record = extract(path, Language::Elixir, source);
        assert_eq!(record.language, Language::Elixir);
        assert_eq!(record.lines, 1);
        assert!(record.symbols.is_empty());
        assert!(record.calls.is_empty());
        assert!(record.field_accesses.is_empty());
        assert_eq!(record.imports.len(), 1);
    }
}
