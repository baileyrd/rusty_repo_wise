//! LCOM4 ("Lack of Cohesion of Methods", variant 4) structural-cohesion
//! marker: per class, builds a graph where methods are nodes and an edge
//! connects two methods that both access at least one common field, then
//! counts connected components. A class whose methods split into 2+
//! disjoint groups has no field overlap between those groups at all —
//! plausibly doing more than one job, and a candidate for splitting.
//!
//! Field-access data only exists for the languages issue #51 scopes:
//! Rust, Python, and TypeScript/JavaScript (see each language's own
//! extraction logic in `repowise-parser` — `field_expression`/
//! `attribute`/`member_expression` nodes on a `self`/`this` receiver).
//! Classes in any other language have an empty `field_accesses` list and
//! are silently skipped here — that's "not enough data", not "zero
//! cohesion", so it must not be reported as a finding either way.
//!
//! Only methods with at least one recorded field access become graph
//! nodes. A method that never touches a class field (a pure delegator, a
//! static-style helper, one that only calls sibling methods) is excluded
//! from the graph entirely rather than counted as its own singleton
//! component — otherwise nearly every real-world class would report
//! "low cohesion" the moment it contains even one such method, which
//! isn't the signal this marker is meant to catch. A class needs at
//! least two field-touching methods before "do they share fields" is
//! even a meaningful question.
//!
//! Connected components are computed with a small hand-rolled
//! union-find rather than a graph-library dependency: per-class method
//! counts are small (dozens at most in practice), so a naive pairwise
//! Set-intersection is simpler than introducing a new dependency for
//! this one cheap computation.

use repowise_core::{FileRecord, Symbol, SymbolKind};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// A class whose tracked methods form at least this many disjoint
/// field-access groups is flagged.
pub const LOW_COHESION_MIN_COMPONENTS: usize = 2;

#[derive(Debug, Clone)]
pub struct LowCohesionCandidate {
    pub file: PathBuf,
    pub class: String,
    pub line: Option<usize>,
    /// Number of disjoint field-access groups (always >= 2 for anything
    /// returned here).
    pub components: usize,
    /// How many of the class's methods had at least one recorded field
    /// access and were therefore included in the graph.
    pub tracked_methods: usize,
}

/// Find every class across `index` whose field-touching methods split
/// into 2+ disjoint groups. See the module doc comment for why methods
/// with no recorded field access are excluded from the graph, and why
/// languages without field-access extraction degrade to "skipped"
/// rather than "flagged" or "cohesive".
pub fn find_low_cohesion(index: &repowise_core::RepoIndex) -> Vec<LowCohesionCandidate> {
    let mut out = Vec::new();
    for file in &index.files {
        out.extend(low_cohesion_in_file(file));
    }
    out.sort_by(|a, b| {
        b.components
            .cmp(&a.components)
            .then(a.file.cmp(&b.file))
            .then(a.class.cmp(&b.class))
    });
    out
}

fn low_cohesion_in_file(file: &FileRecord) -> Vec<LowCohesionCandidate> {
    if file.field_accesses.is_empty() {
        return Vec::new();
    }

    let mut methods_by_class: HashMap<&str, Vec<&Symbol>> = HashMap::new();
    for sym in &file.symbols {
        if sym.kind == SymbolKind::Method {
            if let Some(parent) = &sym.parent {
                methods_by_class
                    .entry(parent.as_str())
                    .or_default()
                    .push(sym);
            }
        }
    }
    if methods_by_class.is_empty() {
        return Vec::new();
    }

    let mut fields_by_method: HashMap<&str, HashSet<&str>> = HashMap::new();
    for fa in &file.field_accesses {
        fields_by_method
            .entry(fa.method.as_str())
            .or_default()
            .insert(fa.field_name.as_str());
    }

    let mut out = Vec::new();
    for (class, methods) in methods_by_class {
        let tracked: Vec<&Symbol> = methods
            .into_iter()
            .filter(|m| fields_by_method.contains_key(m.id.as_str()))
            .collect();
        if tracked.len() < 2 {
            continue;
        }

        let mut uf = UnionFind::new(tracked.len());
        for i in 0..tracked.len() {
            let fields_i = &fields_by_method[tracked[i].id.as_str()];
            for j in (i + 1)..tracked.len() {
                let fields_j = &fields_by_method[tracked[j].id.as_str()];
                if !fields_i.is_disjoint(fields_j) {
                    uf.union(i, j);
                }
            }
        }

        let components = uf.component_count();
        if components < LOW_COHESION_MIN_COMPONENTS {
            continue;
        }

        let line = file
            .symbols
            .iter()
            .find(|s| s.name == class && matches!(s.kind, SymbolKind::Struct | SymbolKind::Class))
            .map(|s| s.start_line);

        out.push(LowCohesionCandidate {
            file: file.path.clone(),
            class: class.to_string(),
            line,
            components,
            tracked_methods: tracked.len(),
        });
    }
    out
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[ra] = rb;
        }
    }

    fn component_count(&mut self) -> usize {
        let roots: HashSet<usize> = (0..self.parent.len()).map(|i| self.find(i)).collect();
        roots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{FieldAccessRef, ImportRef, Language, RepoIndex, SymbolKind};

    fn method(file: &str, class: &str, name: &str, start_line: usize) -> Symbol {
        let path = PathBuf::from(file);
        Symbol {
            id: Symbol::make_id(&path, name, start_line),
            name: name.to_string(),
            kind: SymbolKind::Method,
            file: path,
            start_line,
            end_line: start_line + 1,
            parent: Some(class.to_string()),
            complexity: 1,
            max_nesting_depth: 0,
            bumpy_road_bumps: 0,
            complex_conditionals: Vec::new(),
            param_count: 0,
            primitive_param_count: 0,
            body_hash: None,
        }
    }

    fn class_symbol(file: &str, name: &str, start_line: usize) -> Symbol {
        let path = PathBuf::from(file);
        Symbol {
            id: Symbol::make_id(&path, name, start_line),
            name: name.to_string(),
            kind: SymbolKind::Class,
            file: path,
            start_line,
            end_line: start_line + 20,
            parent: None,
            complexity: 0,
            max_nesting_depth: 0,
            bumpy_road_bumps: 0,
            complex_conditionals: Vec::new(),
            param_count: 0,
            primitive_param_count: 0,
            body_hash: None,
        }
    }

    fn access(method: &Symbol, field: &str) -> FieldAccessRef {
        FieldAccessRef {
            method: method.id.clone(),
            field_name: field.to_string(),
            line: method.start_line,
        }
    }

    fn index_with(file: FileRecord) -> RepoIndex {
        RepoIndex {
            root: PathBuf::from("/fixture"),
            files: vec![file],
            other_files: 0,
        }
    }

    #[test]
    fn flags_a_class_with_two_disjoint_method_groups() {
        let cls = class_symbol("widget.rs", "Widget", 1);
        let read_a = method("widget.rs", "Widget", "read_a", 2);
        let write_a = method("widget.rs", "Widget", "write_a", 3);
        let read_b = method("widget.rs", "Widget", "read_b", 4);
        let write_b = method("widget.rs", "Widget", "write_b", 5);

        let field_accesses = vec![
            access(&read_a, "a"),
            access(&write_a, "a"),
            access(&read_b, "b"),
            access(&write_b, "b"),
        ];

        let file = FileRecord {
            path: PathBuf::from("widget.rs"),
            language: Language::Rust,
            lines: 30,
            symbols: vec![cls, read_a, write_a, read_b, write_b],
            imports: Vec::<ImportRef>::new(),
            calls: Vec::new(),
            field_accesses,
        };

        let candidates = find_low_cohesion(&index_with(file));
        assert_eq!(candidates.len(), 1);
        let c = &candidates[0];
        assert_eq!(c.class, "Widget");
        assert_eq!(c.components, 2);
        assert_eq!(c.tracked_methods, 4);
        assert_eq!(c.line, Some(1));
    }

    #[test]
    fn does_not_flag_a_cohesive_class() {
        let cls = class_symbol("widget.rs", "Widget", 1);
        let read_a = method("widget.rs", "Widget", "read_a", 2);
        let write_a = method("widget.rs", "Widget", "write_a", 3);
        let touches_both = method("widget.rs", "Widget", "touches_both", 4);

        let field_accesses = vec![
            access(&read_a, "a"),
            access(&write_a, "a"),
            access(&touches_both, "a"),
            access(&touches_both, "b"),
        ];

        let file = FileRecord {
            path: PathBuf::from("widget.rs"),
            language: Language::Rust,
            lines: 30,
            symbols: vec![cls, read_a, write_a, touches_both],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses,
        };

        assert!(find_low_cohesion(&index_with(file)).is_empty());
    }

    #[test]
    fn methods_with_no_field_access_are_excluded_not_counted_as_singletons() {
        let cls = class_symbol("widget.rs", "Widget", 1);
        let read_a = method("widget.rs", "Widget", "read_a", 2);
        let write_a = method("widget.rs", "Widget", "write_a", 3);
        // A pure delegator that never touches a field of its own.
        let delegator = method("widget.rs", "Widget", "delegator", 4);

        let field_accesses = vec![access(&read_a, "a"), access(&write_a, "a")];

        let file = FileRecord {
            path: PathBuf::from("widget.rs"),
            language: Language::Rust,
            lines: 30,
            symbols: vec![cls, read_a, write_a, delegator],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses,
        };

        // Only 2 tracked methods (read_a/write_a), both touching "a" ->
        // 1 component -> not flagged. If the excluded delegator were
        // wrongly counted as its own component, this would report 2.
        assert!(find_low_cohesion(&index_with(file)).is_empty());
    }

    #[test]
    fn skips_a_class_with_fewer_than_two_tracked_methods() {
        let cls = class_symbol("widget.rs", "Widget", 1);
        let only = method("widget.rs", "Widget", "only", 2);
        let field_accesses = vec![access(&only, "a")];

        let file = FileRecord {
            path: PathBuf::from("widget.rs"),
            language: Language::Rust,
            lines: 30,
            symbols: vec![cls, only],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses,
        };

        assert!(find_low_cohesion(&index_with(file)).is_empty());
    }

    #[test]
    fn a_language_with_no_field_access_extraction_is_skipped_not_flagged() {
        let cls = class_symbol("widget.go", "Widget", 1);
        let m1 = method("widget.go", "Widget", "m1", 2);
        let m2 = method("widget.go", "Widget", "m2", 3);

        let file = FileRecord {
            path: PathBuf::from("widget.go"),
            language: Language::Go,
            lines: 30,
            symbols: vec![cls, m1, m2],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        };

        assert!(find_low_cohesion(&index_with(file)).is_empty());
    }
}
