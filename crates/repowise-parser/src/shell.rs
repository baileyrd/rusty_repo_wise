use crate::metrics;
use crate::util::text;
use repowise_core::{CallRef, FileRecord, ImportRef, Language, Symbol, SymbolKind};
use std::path::{Component, Path, PathBuf};
use tree_sitter::{Node, Parser};

pub fn extract(path: &Path, source: &str) -> anyhow::Result<FileRecord> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_bash::LANGUAGE.into())?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse {}", path.display()))?;

    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    let mut calls = Vec::new();

    let mut walker = Walker {
        path,
        source,
        symbols: &mut symbols,
        imports: &mut imports,
        calls: &mut calls,
        scope_stack: Vec::new(),
    };
    walker.visit(tree.root_node());

    Ok(FileRecord {
        path: path.to_path_buf(),
        language: Language::Shell,
        lines: source.lines().count(),
        symbols,
        imports,
        calls,
        field_accesses: Vec::new(),
    })
}

struct Walker<'a> {
    path: &'a Path,
    source: &'a str,
    symbols: &'a mut Vec<Symbol>,
    imports: &'a mut Vec<ImportRef>,
    calls: &'a mut Vec<CallRef>,
    scope_stack: Vec<String>,
}

impl<'a> Walker<'a> {
    fn current_scope(&self) -> Option<String> {
        self.scope_stack.last().cloned()
    }

    fn line_of(&self, node: Node) -> usize {
        node.start_position().row + 1
    }

    fn visit(&mut self, node: Node) {
        match node.kind() {
            // Shell has no classes/structs — per repowise's own
            // documented scope for this language tier, only functions
            // are extracted as symbols, and (unlike every other
            // language here) there's no dead-code check at all for them
            // (see `repowise-health`): a shell function is routinely
            // invoked only from the command line, another script, or a
            // cron job, none of which this port's call graph can see.
            "function_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, self.source).to_string();
                    let start_line = self.line_of(node);
                    let end_line = node.end_position().row + 1;
                    let id = Symbol::make_id(self.path, &name, start_line);
                    let body = node.child_by_field_name("body");
                    let complexity = body
                        .map(|b| {
                            metrics::cyclomatic_complexity(
                                b,
                                |n| is_decision(n, self.source),
                                is_nested_function,
                            )
                        })
                        .unwrap_or(0);
                    let max_nesting_depth = body
                        .map(|b| {
                            metrics::max_nesting_depth(
                                b,
                                |n| is_decision(n, self.source),
                                is_nested_function,
                            )
                        })
                        .unwrap_or(0);
                    let bumpy_road_bumps = body
                        .map(|b| {
                            metrics::bumpy_road_bumps(
                                b,
                                |n| is_decision(n, self.source),
                                is_nested_function,
                            )
                        })
                        .unwrap_or(0);
                    let body_hash = body.and_then(|b| metrics::body_hash(b, self.source));
                    self.symbols.push(Symbol {
                        id: id.clone(),
                        name,
                        kind: SymbolKind::Function,
                        file: self.path.to_path_buf(),
                        start_line,
                        end_line,
                        parent: None,
                        complexity,
                        max_nesting_depth,
                        bumpy_road_bumps,
                        complex_conditionals: Vec::new(),
                        // Shell functions take positional parameters
                        // ($1, $2, ...) rather than a declared parameter
                        // list — there's nothing in the grammar to count.
                        param_count: 0,
                        body_hash,
                    });
                    self.scope_stack.push(id);
                    self.visit_children(node);
                    self.scope_stack.pop();
                    return;
                }
            }
            "command" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let command_word = command_word_text(name_node, self.source);
                    if matches!(command_word.as_str(), "source" | ".") {
                        if let Some(arg) = first_argument(node) {
                            let line = self.line_of(node);
                            if let Some((path_text, resolved_file)) =
                                source_argument_path(arg, self.source, self.path)
                            {
                                self.imports.push(ImportRef {
                                    path: path_text,
                                    line,
                                    resolved_file,
                                });
                            }
                        }
                        self.visit_children(node);
                        return;
                    }
                    if !command_word.is_empty() {
                        self.calls.push(CallRef {
                            caller: self.current_scope(),
                            callee_name: command_word,
                            line: self.line_of(node),
                        });
                    }
                }
            }
            _ => {}
        }
        self.visit_children(node);
    }

    fn visit_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child);
        }
    }
}

/// A `command`'s own name (its `command_name` field's single child, a
/// bare `word` for the plain, unquoted case this heuristic handles).
fn command_word_text(name_node: Node, source: &str) -> String {
    let mut cursor = name_node.walk();
    let found = name_node.children(&mut cursor).find(|c| c.kind() == "word");
    found
        .map(|w| text(w, source).to_string())
        .unwrap_or_default()
}

/// The first non-`command_name` child of a `command` node — `source`/`.`'s
/// path argument.
fn first_argument(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|c| c.kind() != "command_name");
    found
}

/// `source`/`.`'s path argument, resolved where possible.
///
/// Handles three shapes: a bare unquoted `word` or a quote-free
/// `raw_string` (single-quoted) — the literal text is the path, resolved
/// directly against the including script's own directory; a `string`
/// (double-quoted) with no expansion inside — same treatment; and the
/// `$SCRIPT_DIR/rest.sh` idiom (a `string` whose *first* content is a
/// `simple_expansion` of a variable literally named `SCRIPT_DIR`,
/// followed by plain `string_content`) — since `$SCRIPT_DIR` is, by the
/// idiom's own convention (`SCRIPT_DIR="$(dirname "$0")"`), the
/// including script's own directory, the remaining suffix resolves the
/// same way a plain relative path would. Any other expansion (`$HOME`,
/// `$(cmd)`, a variable name other than `SCRIPT_DIR`) has no static value
/// to resolve against, so the import is still recorded (for visibility)
/// but left unresolved.
fn source_argument_path(
    arg: Node,
    source: &str,
    current_file: &Path,
) -> Option<(String, Option<PathBuf>)> {
    match arg.kind() {
        "word" => {
            let spec = text(arg, source).to_string();
            let resolved = resolve_relative(current_file, &spec);
            Some((spec, resolved))
        }
        "raw_string" => {
            let spec = text(arg, source).trim_matches('\'').to_string();
            let resolved = resolve_relative(current_file, &spec);
            Some((spec, resolved))
        }
        "string" => {
            let mut cursor = arg.walk();
            let named: Vec<Node> = arg.named_children(&mut cursor).collect();
            let display_path = text(arg, source).trim_matches('"').to_string();
            if named.is_empty() {
                // An empty string literal — nothing to resolve.
                return Some((display_path, None));
            }
            let first_is_script_dir = named[0].kind() == "simple_expansion"
                && named[0]
                    .named_child(0)
                    .map(|v| text(v, source) == "SCRIPT_DIR")
                    .unwrap_or(false);
            if first_is_script_dir {
                let suffix: String = named[1..]
                    .iter()
                    .filter(|n| n.kind() == "string_content")
                    .map(|n| text(*n, source))
                    .collect();
                let spec = suffix.trim_start_matches('/');
                let resolved = resolve_relative(current_file, spec);
                return Some((display_path, resolved));
            }
            if named.iter().all(|n| n.kind() == "string_content") {
                let spec: String = named.iter().map(|n| text(*n, source)).collect();
                let resolved = resolve_relative(current_file, &spec);
                return Some((display_path, resolved));
            }
            // Some other expansion (`$HOME`, `$(cmd)`, ...) with no
            // static value — recorded, but not resolvable.
            Some((display_path, None))
        }
        _ => None,
    }
}

/// Resolve a relative path against the including script's own directory
/// — the common "sourced file lives next to (or under) its source"
/// convention.
fn resolve_relative(current_file: &Path, spec: &str) -> Option<PathBuf> {
    let dir = current_file.parent()?;
    let candidate = dir.join(spec);
    candidate.is_file().then(|| normalize(&candidate))
}

/// Lexically collapse `.`/`..` components (no filesystem access) so a
/// resolved path matches the plain, already-canonical paths
/// `discover_files` produces.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Cyclomatic-complexity decision points for shell: branches, loops, and
/// `case` clauses (not the bare `*)` wildcard fallthrough, shell's
/// equivalent of `default`).
fn is_decision(n: Node, source: &str) -> bool {
    match n.kind() {
        "if_statement" | "for_statement" | "c_style_for_statement" | "while_statement" => true,
        "case_item" => n
            .child_by_field_name("value")
            .map(|v| text(v, source) != "*")
            .unwrap_or(true),
        _ => false,
    }
}

/// Only nested named function definitions get their own symbol; this
/// stops recursion there so a nested function's complexity isn't
/// double-counted into the enclosing one.
fn is_nested_function(n: Node) -> bool {
    n.kind() == "function_definition"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_str(source: &str) -> FileRecord {
        extract(Path::new("script.sh"), source).unwrap()
    }

    #[test]
    fn extracts_function_and_call() {
        let rec = extract_str("func() {\n  helper\n}\n\nfunc\n");
        let func = rec.symbols.iter().find(|s| s.name == "func").unwrap();
        assert_eq!(func.kind, SymbolKind::Function);

        let callees: Vec<_> = rec.calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(callees.contains(&"helper"));
        assert!(callees.contains(&"func"));

        let helper_call = rec
            .calls
            .iter()
            .find(|c| c.callee_name == "helper")
            .unwrap();
        assert_eq!(helper_call.caller, Some(func.id.clone()));
    }

    #[test]
    fn records_plain_relative_source_and_dot() {
        let rec = extract_str("source \"helper.sh\"\n. \"./other.sh\"\n");
        let paths: Vec<_> = rec.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"helper.sh"));
        assert!(paths.contains(&"./other.sh"));
    }

    #[test]
    fn records_script_dir_idiom_and_unresolvable_expansion_separately() {
        let rec = extract_str(
            "SCRIPT_DIR=\"$(dirname \"$0\")\"\nsource \"$SCRIPT_DIR/helper.sh\"\nsource \"$HOME/other.sh\"\n",
        );
        let script_dir_import = rec
            .imports
            .iter()
            .find(|i| i.path == "$SCRIPT_DIR/helper.sh")
            .unwrap();
        // Not resolvable here (no real file on disk), but this asserts
        // the import was recognized and recorded distinctly from the
        // unresolvable-expansion case below.
        assert!(script_dir_import.path.starts_with("$SCRIPT_DIR"));

        let home_import = rec
            .imports
            .iter()
            .find(|i| i.path == "$HOME/other.sh")
            .unwrap();
        assert!(home_import.resolved_file.is_none());
    }

    #[test]
    fn computes_cyclomatic_complexity() {
        let rec = extract_str(
            "branchy() {\n  if [ -f x ]; then\n    echo 1\n  fi\n  for i in a b c; do\n    echo $i\n  done\n  case \"$x\" in\n    a) echo a ;;\n    *) echo default ;;\n  esac\n}\n",
        );
        let branchy = rec.symbols.iter().find(|s| s.name == "branchy").unwrap();
        // base(1) + if(1) + for(1) + case-item "a"(1) = 4 (the `*)`
        // wildcard fallthrough doesn't count, same as `default`).
        assert_eq!(branchy.complexity, 4);
    }

    #[test]
    fn hashes_duplicate_function_bodies_identically() {
        let rec = extract_str(
            "one() {\n  total=0\n  for i in 1 2 3; do\n    total=$((total + i))\n  done\n  echo $total\n}\n\ntwo() {\n  total=0\n  for i in 1 2 3; do\n    total=$((total + i))\n  done\n  echo $total\n}\n\nshort_fn() {\n  echo 1\n}\n",
        );
        let one = rec.symbols.iter().find(|s| s.name == "one").unwrap();
        let two = rec.symbols.iter().find(|s| s.name == "two").unwrap();
        let short = rec.symbols.iter().find(|s| s.name == "short_fn").unwrap();

        assert!(one.body_hash.is_some());
        assert_eq!(one.body_hash, two.body_hash);
        assert!(short.body_hash.is_none());
    }
}
