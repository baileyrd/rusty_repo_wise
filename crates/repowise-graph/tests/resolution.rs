//! End-to-end tests: real files on disk -> discover -> parse -> build graph.
//! Exercises the directory-layout-based module resolution in `modpath`,
//! which needs actual files (Rust resolution walks up to find `Cargo.toml`).

use repowise_core::{discover_files, FileRecord, Language, RepoIndex};
use repowise_graph::RepoGraph;
use std::fs;
use std::path::Path;

fn index_dir(root: &Path) -> RepoIndex {
    let discovered = discover_files(root).unwrap();
    let mut files = Vec::new();
    let mut other_files = 0;
    for entry in discovered {
        if matches!(entry.language, Language::Other) {
            other_files += 1;
            continue;
        }
        let source = fs::read_to_string(&entry.path).unwrap();
        match repowise_parser::parse_file(&entry.path, entry.language, &source).unwrap() {
            Some(record) => files.push(record),
            None => other_files += 1,
        }
    }
    RepoIndex {
        root: root.to_path_buf(),
        files,
        other_files,
    }
}

fn find_file<'a>(index: &'a RepoIndex, suffix: &str) -> &'a FileRecord {
    index
        .files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("no indexed file ending in {suffix}"))
}

#[test]
fn resolves_python_imports_and_calls() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("pkg")).unwrap();
    fs::write(root.join("pkg/__init__.py"), "").unwrap();
    fs::write(
        root.join("pkg/utils.py"),
        "def helper(x):\n    return x + 1\n\nclass Widget:\n    def render(self):\n        return helper(1)\n",
    )
    .unwrap();
    fs::write(
        root.join("main.py"),
        "from pkg.utils import helper, Widget\nimport os.path\n\ndef run():\n    w = Widget()\n    return helper(2)\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_py = find_file(&index, "main.py").path.clone();
    let utils_py = find_file(&index, "utils.py").path.clone();

    let deps = graph.dependencies_of(&main_py);
    assert!(
        deps.contains(&utils_py),
        "expected main.py to depend on utils.py, got {deps:?}"
    );

    let dependents = graph.dependents_of(&utils_py);
    assert!(
        dependents.contains(&main_py),
        "expected utils.py to be depended on by main.py, got {dependents:?}"
    );

    // `import os.path` has no matching file in this tiny fixture, so it
    // must not resolve to anything (and should count as unresolved).
    assert!(graph.unresolved_imports >= 1);

    let helper_matches = graph.search("helper");
    assert_eq!(helper_matches.len(), 1);
}

#[test]
fn resolves_rust_module_imports_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "mod foo;\n\nfn top() -> i32 {\n    foo::bar()\n}\n",
    )
    .unwrap();
    fs::write(root.join("src/foo.rs"), "pub fn bar() -> i32 { 42 }\n").unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let lib_rs = find_file(&index, "lib.rs").path.clone();
    let foo_rs = find_file(&index, "foo.rs").path.clone();

    // `mod foo;` is resolved directly via Rust's file-layout convention
    // (see `resolve_mod_file` in repowise-parser), independent of `use`.
    let deps = graph.dependencies_of(&lib_rs);
    assert!(
        deps.contains(&foo_rs),
        "expected lib.rs to depend on foo.rs via `mod foo;`, got {deps:?}"
    );

    let matches = graph.search("bar");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file, foo_rs);
}

#[test]
fn resolves_typescript_relative_imports_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/utils.ts"),
        "export function helper(x: number): number {\n  return x + 1;\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.ts"),
        "import { helper } from \"./utils\";\nimport \"left-pad\";\n\nexport function run(): number {\n  return helper(1);\n}\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_ts = find_file(&index, "main.ts").path.clone();
    let utils_ts = find_file(&index, "utils.ts").path.clone();

    // The extensionless relative import is resolved directly at parse
    // time (see `resolve_relative_import` in `repowise-parser`), same as
    // Rust's `mod foo;`.
    let deps = graph.dependencies_of(&main_ts);
    assert!(
        deps.contains(&utils_ts),
        "expected main.ts to depend on utils.ts, got {deps:?}"
    );

    // A bare (npm-package) specifier has no `node_modules` to resolve
    // against, so it must count as unresolved rather than silently drop.
    assert!(graph.unresolved_imports >= 1);

    let matches = graph.search("helper");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file, utils_ts);
}

#[test]
fn resolves_java_package_imports_via_maven_source_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let src_root = root.join("src/main/java/com/example/util");
    fs::create_dir_all(&src_root).unwrap();
    fs::write(
        src_root.join("Helper.java"),
        "package com.example.util;\n\npublic class Helper {\n  public static int compute(int x) {\n    return x + 1;\n  }\n}\n",
    )
    .unwrap();
    let app_root = root.join("src/main/java/com/example/app");
    fs::create_dir_all(&app_root).unwrap();
    fs::write(
        app_root.join("Main.java"),
        "package com.example.app;\n\nimport com.example.util.Helper;\n\npublic class Main {\n  public int run() {\n    return Helper.compute(1);\n  }\n}\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_java = find_file(&index, "Main.java").path.clone();
    let helper_java = find_file(&index, "Helper.java").path.clone();

    // Resolved via the `src/main/java`-anchored package path, not a
    // repo-root-relative one (which would incorrectly require
    // `src.main.java.com.example.util.Helper`).
    let deps = graph.dependencies_of(&main_java);
    assert!(
        deps.contains(&helper_java),
        "expected Main.java to depend on Helper.java, got {deps:?}"
    );

    let matches = graph.search("compute");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file, helper_java);
}

#[test]
fn resolves_go_imports_via_go_mod_anchored_module_path() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("go.mod"), "module example.com/mymod\n\ngo 1.22\n").unwrap();
    fs::create_dir_all(root.join("util")).unwrap();
    fs::write(
        root.join("util/helper.go"),
        "package util\n\nfunc Compute(x int) int {\n\treturn x + 1\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("main.go"),
        "package main\n\nimport \"example.com/mymod/util\"\n\nfunc run() int {\n\treturn util.Compute(1)\n}\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_go = find_file(&index, "main.go").path.clone();
    let helper_go = find_file(&index, "util/helper.go").path.clone();

    // Resolved via the `go.mod`-anchored module path
    // (`example.com/mymod/util`), not a repo-root-relative one.
    let deps = graph.dependencies_of(&main_go);
    assert!(
        deps.contains(&helper_go),
        "expected main.go to depend on util/helper.go, got {deps:?}"
    );

    let matches = graph.search("Compute");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file, helper_go);
}

#[test]
fn resolves_kotlin_imports_including_across_a_mixed_java_kotlin_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    // A Java class under the Maven/Gradle Java source root...
    let java_src_root = root.join("src/main/java/com/example/util");
    fs::create_dir_all(&java_src_root).unwrap();
    fs::write(
        java_src_root.join("Helper.java"),
        "package com.example.util;\n\npublic class Helper {\n  public static int compute(int x) {\n    return x + 1;\n  }\n}\n",
    )
    .unwrap();

    // ...imported from a Kotlin file under the sibling Kotlin source root.
    let kotlin_src_root = root.join("src/main/kotlin/com/example/app");
    fs::create_dir_all(&kotlin_src_root).unwrap();
    fs::write(
        kotlin_src_root.join("Main.kt"),
        "package com.example.app\n\nimport com.example.util.Helper\n\nclass Main {\n  fun run(): Int {\n    return Helper.compute(1)\n  }\n}\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_kt = find_file(&index, "Main.kt").path.clone();
    let helper_java = find_file(&index, "Helper.java").path.clone();

    // Resolved via the shared JVM package-path convention across both
    // `src/main/java` and `src/main/kotlin` source roots.
    let deps = graph.dependencies_of(&main_kt);
    assert!(
        deps.contains(&helper_java),
        "expected Main.kt to depend on Helper.java, got {deps:?}"
    );

    let matches = graph.search("compute");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file, helper_java);
}

#[test]
fn resolves_cpp_quote_includes_but_not_angle_includes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    // `.hpp`, not `.h` — `.h` is deliberately left as `Language::Other`
    // (ambiguous with a future plain-C extractor), so it isn't indexed.
    fs::write(root.join("helper.hpp"), "int compute(int x);\n").unwrap();
    fs::write(
        root.join("main.cpp"),
        "#include \"helper.hpp\"\n#include <vector>\n\nint run() {\n    return compute(1);\n}\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_cpp = find_file(&index, "main.cpp").path.clone();
    let helper_hpp = find_file(&index, "helper.hpp").path.clone();

    let deps = graph.dependencies_of(&main_cpp);
    assert!(
        deps.contains(&helper_hpp),
        "expected main.cpp to depend on helper.hpp, got {deps:?}"
    );

    // `<vector>` has no include-path search list to resolve against and
    // must count as unresolved rather than silently dropped.
    assert!(graph.unresolved_imports >= 1);
}

#[test]
fn resolves_csharp_usings_via_folder_to_namespace_heuristic() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("App/Util")).unwrap();
    fs::write(
        root.join("App/Util/Helper.cs"),
        "namespace App.Util {\n  public class Helper {\n    public static int Compute(int x) {\n      return x + 1;\n    }\n  }\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("Main.cs"),
        "using App.Util;\n\nclass Program {\n  int Run() {\n    return Helper.Compute(1);\n  }\n}\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_cs = find_file(&index, "Main.cs").path.clone();
    let helper_cs = find_file(&index, "App/Util/Helper.cs").path.clone();

    // `using App.Util;` resolves via the folder-path-mirrors-namespace
    // heuristic, not a repo-root-relative file path.
    let deps = graph.dependencies_of(&main_cs);
    assert!(
        deps.contains(&helper_cs),
        "expected Main.cs to depend on App/Util/Helper.cs, got {deps:?}"
    );

    let matches = graph.search("Compute");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file, helper_cs);
}

#[test]
fn resolves_scala_package_imports_via_sbt_source_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let src_root = root.join("src/main/scala/com/example/util");
    fs::create_dir_all(&src_root).unwrap();
    fs::write(
        src_root.join("Helper.scala"),
        "package com.example.util\n\nobject Helper {\n  def compute(x: Int): Int = {\n    x + 1\n  }\n}\n",
    )
    .unwrap();
    let app_root = root.join("src/main/scala/com/example/app");
    fs::create_dir_all(&app_root).unwrap();
    fs::write(
        app_root.join("Main.scala"),
        "package com.example.app\n\nimport com.example.util.Helper\n\nclass Main {\n  def run(): Int = {\n    Helper.compute(1)\n  }\n}\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_scala = find_file(&index, "Main.scala").path.clone();
    let helper_scala = find_file(&index, "Helper.scala").path.clone();

    // Resolved via the `src/main/scala`-anchored package path (the same
    // `jvm_module_path` convention shared with Java/Kotlin), not a
    // repo-root-relative one.
    let deps = graph.dependencies_of(&main_scala);
    assert!(
        deps.contains(&helper_scala),
        "expected Main.scala to depend on Helper.scala, got {deps:?}"
    );

    let matches = graph.search("compute");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file, helper_scala);
}

#[test]
fn resolves_ruby_require_relative_but_not_plain_require() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("helper.rb"),
        "class Helper\n  def self.compute(x)\n    x + 1\n  end\nend\n",
    )
    .unwrap();
    fs::write(
        root.join("main.rb"),
        "require_relative \"helper\"\nrequire \"json\"\n\ndef run\n  Helper.compute(1)\nend\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_rb = find_file(&index, "main.rb").path.clone();
    let helper_rb = find_file(&index, "helper.rb").path.clone();

    let deps = graph.dependencies_of(&main_rb);
    assert!(
        deps.contains(&helper_rb),
        "expected main.rb to depend on helper.rb, got {deps:?}"
    );

    // `require "json"` is gem-based ($LOAD_PATH-relative) with no static
    // equivalent to resolve against, so it must count as unresolved.
    assert!(graph.unresolved_imports >= 1);
}

#[test]
fn resolves_c_quote_includes_of_recognized_extensions_but_not_conventional_h_headers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    // `.h` is deliberately unmapped to any language (kept as
    // `Language::Other`, same ambiguity-with-C++ call already made for
    // the C++ extractor), so this is never indexed — meaning a
    // conventional `#include "helper.h"` split can't resolve to a real
    // graph edge in this port, a known, stated limitation.
    fs::write(root.join("helper.h"), "int compute(int x);\n").unwrap();
    // A `.c` file, on the other hand, IS a recognized extension, so
    // quote-form `#include` of one resolves directly against the
    // filesystem exactly like C++'s `.hpp` case.
    fs::write(
        root.join("helper_impl.c"),
        "int compute(int x) {\n    return x + 1;\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("main.c"),
        "#include \"helper.h\"\n#include \"helper_impl.c\"\n\nint run(void) {\n    return compute(1);\n}\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_c = find_file(&index, "main.c").path.clone();
    let helper_impl_c = find_file(&index, "helper_impl.c").path.clone();

    let deps = graph.dependencies_of(&main_c);
    assert!(
        deps.contains(&helper_impl_c),
        "expected main.c to depend on helper_impl.c, got {deps:?}"
    );

    // `#include "helper.h"` resolves against the filesystem fine at parse
    // time, but since `.h` files are never indexed as graph nodes, the
    // edge can't be created — it must count as unresolved.
    assert!(graph.unresolved_imports >= 1);
}
