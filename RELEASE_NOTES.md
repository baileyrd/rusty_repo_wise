# Release Notes

Notable changes to this repo, newest first. No tagged releases yet, so entries
are keyed by PR (or by commit, for the two prior changes that predate this
repo routing work through PRs).

---

## PR #91 — Add Swift language support
**2026-07-23** · [#91](https://github.com/baileyrd/rusty_repo_wise/pull/91) · closes [#37](https://github.com/baileyrd/rusty_repo_wise/issues/37)

- **Added:** a `repowise-parser` extractor for Swift — classes/structs/
  enums/actors (all share one `class_declaration` grammar node,
  distinguished by its `declaration_kind` field) map to
  `Class`/`Struct`/`Enum`/`Class`; protocols map to `Trait`. Extensions
  re-open an existing type rather than declaring a new one, so they
  don't get their own symbol, but their name is still pushed onto the
  `class_stack` so extension methods are correctly attributed to the
  extended type. Protocol method requirements have no body at all (a
  distinct `protocol_function_declaration` node, not
  `function_declaration` with an absent body) — recorded as symbols
  with 0 complexity, same treatment as Java/Kotlin/Scala's bodiless
  methods.
- **Import resolution, by design:** Swift's `import` is module-level
  (`import Foundation`), not file-level — there's no per-file
  relative-import syntax and a module name has no file mapping without
  a full build graph. Imports are recorded (for visibility/stats) but
  always left unresolved by design, asserted directly by this PR's own
  graph-layer test rather than treated as a "resolves" case that
  happens to fail.
- **Notable grammar quirk:** unlike every other language done so far,
  Swift's `function_declaration` has no wrapping parameters-list node
  at all — `parameter` nodes are direct children of the function
  declaration itself, interspersed with its name/return-type/body.
  `param_count` is counted directly rather than via the shared
  `metrics::count_params` helper, which assumes a dedicated list node
  (using that helper here would have silently counted every child, not
  just parameters).
- **Dependency note:** pins `tree-sitter-swift = "0.6"` rather than the
  newer 0.7.x release — 0.7.3's grammar targets ABI 15, incompatible
  with this workspace's tree-sitter 0.24 core (ABI 13–14 only). 0.6.0
  is ABI-compatible, the same fix already applied to
  `tree-sitter-c-sharp`/`tree-sitter-c`.
- 6 new tests (class/struct/protocol/method extraction, extension-
  attribution-without-duplicate-symbol, module-import-stays-unresolved,
  bare/member call tracking, cyclomatic complexity, duplicate-body
  hashing) plus a `repowise-graph` end-to-end test proving module
  imports correctly stay unresolved; 100 tests passing workspace-wide.
  Tenth language merged out of this session's `parity-loop`
  gap-analysis pass (after TypeScript/JavaScript in #26, Java in #75,
  Kotlin in #77, Go in #79, C++ in #81, C# in #83, Scala in #85, Ruby
  in #87, and C in #89) — next up per the loop is PHP (#38).

## PR #89 — Add C language support
**2026-07-23** · [#89](https://github.com/baileyrd/rusty_repo_wise/pull/89) · closes [#36](https://github.com/baileyrd/rusty_repo_wise/issues/36)

- **Added:** a `repowise-parser` extractor for C — functions and structs
  (`SymbolKind::Function`/`Struct`). Simpler than C++'s: plain C has no
  member functions at all, so there's no `class_stack` — struct fields
  and function bodies never nest into each other. Quote-form
  `#include "local.h"` is resolved directly against the filesystem at
  parse time (mirroring C++'s own `resolve_include`); angle-form
  `#include <system>` stays unresolved by design.
- **Design decision, left open by #32:** the C/C++ `.h` ambiguity.
  `.h` stays unmapped to either language (`Language::Other`) — the same
  call already made for C++'s own extension set — rather than guessing
  via syntax-sniffing. This has a **more significant practical
  consequence for C than it did for C++**: C++ has alternate,
  unambiguous header extensions (`.hpp`/`.hh`/`.hxx`) commonly used in
  practice, but C conventionally uses `.h` for nearly all its headers
  with no alternate in common use — so a conventional
  `#include "foo.h"` split resolves against the filesystem fine at parse
  time, but never becomes a real graph edge, since the header itself is
  never indexed as a graph node. Demonstrated directly by this PR's own
  graph resolution test (asserted, not just described in prose).
- **Dependency note:** pins `tree-sitter-c = "0.21"` rather than the
  newer 0.24.x release — 0.24.2's grammar targets ABI 15, incompatible
  with this workspace's tree-sitter 0.24 core (ABI 13–14 only). 0.21
  predates the `LanguageFn` API and is ABI-compatible, the same fix
  already applied to `tree-sitter-c-sharp`.
- 5 new tests (struct/function extraction, quote/angle include
  handling, field/bare call tracking, cyclomatic complexity,
  duplicate-body hashing) plus a `repowise-graph` end-to-end test
  proving quote-form includes of recognized extensions resolve while
  conventional `.h` headers stay unresolved; 94 tests passing
  workspace-wide. Ninth language merged out of this session's
  `parity-loop` gap-analysis pass (after TypeScript/JavaScript in #26,
  Java in #75, Kotlin in #77, Go in #79, C++ in #81, C# in #83, Scala in
  #85, and Ruby in #87) — next up per the loop is Swift (#37).

## PR #87 — Add Ruby language support
**2026-07-23** · [#87](https://github.com/baileyrd/rusty_repo_wise/pull/87) · closes [#35](https://github.com/baileyrd/rusty_repo_wise/issues/35)

- **Added:** a `repowise-parser` extractor for Ruby — classes and
  modules (mapped to `Class`/`Module`), plus `def` methods (both
  instance and `def self.`-style class methods), nested via a
  `class_stack` the same way Java/Kotlin/Scala do. `require_relative` is
  resolved directly against the filesystem at parse time (mirroring
  TS/JS's relative-import resolution and C++'s quote-form `#include`),
  trying the exact path then appending a `.rb` extension; plain
  `require` is gem-based (`$LOAD_PATH`) with no static equivalent to
  resolve against, so it's recorded but left unresolved by design.
  `receiver.new` calls are recorded as a call to the receiver class
  itself (Ruby's equivalent of `new Type()`).
- **Notable grammar quirk, caught by its own test:** `tree-sitter-ruby`
  names several rules after their own bare keyword (`if`, `elsif`,
  `while`, `until`, `for`, `rescue`, `when`) and *also* keeps that
  keyword as an anonymous child token of the identical kind string —
  double-counting cyclomatic complexity until an `is_named()` guard was
  added to `is_decision`.
- **Known limitation, stated plainly:** bare parenless/argless method
  calls (`helper` with no receiver, parens, or args) aren't
  distinguishable from local variable references by the grammar itself,
  so they aren't recorded as calls — callers should use explicit parens
  (`helper()`) for a call to be tracked.
- 5 new tests (class/module/method extraction, `require_relative`/
  `require` handling, constructor-call tracking, cyclomatic complexity,
  duplicate-body hashing) plus a `repowise-graph` end-to-end test
  proving `require_relative` resolves while plain `require` stays
  unresolved; 89 tests passing workspace-wide. Eighth language merged
  out of this session's `parity-loop` gap-analysis pass (after
  TypeScript/JavaScript in #26, Java in #75, Kotlin in #77, Go in #79,
  C++ in #81, C# in #83, and Scala in #85) — next up per the loop is C
  (#36).

## PR #85 — Add Scala language support
**2026-07-23** · [#85](https://github.com/baileyrd/rusty_repo_wise/pull/85) · closes [#34](https://github.com/baileyrd/rusty_repo_wise/issues/34)

- **Added:** a `repowise-parser` extractor for Scala — classes, objects,
  and traits (mapped to `Class`/`Class`/`Trait`), plus `def` methods.
  Like Java/Kotlin (and unlike Go/C++), Scala methods are always
  declared directly inside their type's `template_body`, so scoping
  uses the same `class_stack` push/pop pattern. Bodiless `def`
  signatures (abstract methods in traits) parse as a distinct
  `function_declaration` node rather than `function_definition` with an
  absent body — both are handled and recorded as symbols with 0
  complexity, same treatment as Java/Kotlin's bodiless methods. `import`
  declarations are extracted (plain and wildcard `_` forms); call and
  `new`-style instance expressions are tracked as calls.
- **Known limitation, stated plainly:** grouped selector imports
  (`import foo.{Bar, Baz}`) resolve to the enclosing package (`foo`)
  rather than being expanded into one entry per selector — an accepted
  simplification, same tradeoff already made for other languages'
  wildcard imports. Curried multi-parameter-list `def`s
  (`def f(a: Int)(b: Int)`) only have their first parameter list
  counted toward `param_count`.
- **Dependency note:** `tree-sitter-scala = "0.23"` turned out to be
  ABI-compatible with this workspace's tree-sitter 0.24 core without
  any downgrade — unlike `tree-sitter-c-sharp`, which needed pinning to
  0.21 (see the #83 entry below).
- Reuses the shared `jvm_module_path` convention from Java/Kotlin for
  import resolution, extended with `src/main/scala`/`src/test/scala` as
  recognized sbt source roots — a mixed Java/Kotlin/Scala project
  resolves imports across all three.
- 6 new tests (class/trait/object/method extraction, plain/wildcard
  imports, object-creation calls, cyclomatic complexity, duplicate-body
  hashing, trait-method-signature handling) plus a `repowise-graph`
  end-to-end test proving sbt-layout package resolution; 84 tests
  passing workspace-wide. Seventh language merged out of this session's
  `parity-loop` gap-analysis pass (after TypeScript/JavaScript in #26,
  Java in #75, Kotlin in #77, Go in #79, C++ in #81, and C# in #83) —
  next up per the loop is Ruby (#35).

## PR #83 — Add C# language support
**2026-07-23** · [#83](https://github.com/baileyrd/rusty_repo_wise/pull/83) · closes [#33](https://github.com/baileyrd/rusty_repo_wise/issues/33)

- **Added:** a `repowise-parser` extractor for C# — classes, structs,
  interfaces, methods, and constructors. Unlike Go/C++, C# methods are
  always declared directly inside their type's body, so scoping uses
  the same `class_stack` push/pop pattern already established for
  Java/Kotlin. `using` directives are extracted as imports
  (plain/dotted/aliased forms handled, `using static` skipped);
  invocation and object-creation expressions are tracked as calls.
- **Dependency note:** pins `tree-sitter-c-sharp = "0.21"` rather than a
  newer 0.23.x release — 0.23.5's grammar targets ABI 15, incompatible
  with this workspace's tree-sitter 0.24 core (ABI 13–14 only). 0.21
  predates the `LanguageFn` API and is ABI-compatible, the same
  workaround pattern used transiently for Rust/Python early in this
  project.
- **Known limitation, stated plainly:** namespace resolution
  (`csharp_namespace_path`) is a folder-mirrors-namespace heuristic —
  nothing in C#/.NET enforces that convention the way Maven/Gradle or
  `go.mod` do for Java/Kotlin/Go, so a project that doesn't follow it
  won't resolve correctly. Like Go, it's keyed by directory rather than
  per-file, so multiple files sharing one namespace resolve to
  whichever was indexed last.
- 6 new tests (class/interface/method extraction, using-directive
  forms, object-creation calls, cyclomatic complexity, duplicate-body
  hashing, interface-signature-vs-real-method) plus a `repowise-graph`
  end-to-end test proving folder-based namespace resolution; 78 tests
  passing workspace-wide. Sixth language merged out of this session's
  `parity-loop` gap-analysis pass (after TypeScript/JavaScript in #26,
  Java in #75, Kotlin in #77, Go in #79, and C++ in #81) — next up per
  the loop is Scala (#34).

## PR #81 — Add C++ language support
**2026-07-23** · [#81](https://github.com/baileyrd/rusty_repo_wise/pull/81) · closes [#32](https://github.com/baileyrd/rusty_repo_wise/issues/32)

- **Added:** a `repowise-parser` extractor for C++ — classes, structs,
  functions, and methods. Like Go, out-of-class method definitions
  (`Ret Widget::area() {...}`) get their parent read directly from the
  qualified name's scope; unlike Go, in-class method *prototypes*
  (`int area();` inside the class body, no bodies) are also tracked via
  a `class_stack`, recorded as separate `Method` symbols — the same
  dual-symbol pattern already established for Java/Kotlin/Go interface
  signatures. Quote-form `#include "local.h"` is resolved directly
  against the filesystem (mirroring TS/JS's relative-import resolution);
  angle-form `#include <system>` has no include-path search list and
  stays unresolved by design.
- **Known limitation, stated plainly:** `.h` is deliberately left
  unmapped to any language (`Language::Other`) — it's ambiguous between
  C and C++, and this issue is C++-only (plain C is tracked separately
  as issue #36). Only unambiguous C++ extensions (`.cpp`/`.cc`/`.cxx`/
  `.hpp`/`.hh`/`.hxx`) are recognized for now.
- 5 new tests (class/prototype/out-of-class-definition extraction,
  quote/angle include handling, member/bare/qualified call tracking,
  cyclomatic complexity, duplicate-body hashing) plus a `repowise-graph`
  end-to-end test proving quote-includes resolve while angle-includes
  stay unresolved; 71 tests passing workspace-wide. Fifth language
  merged out of this session's `parity-loop` gap-analysis pass (after
  TypeScript/JavaScript in #26, Java in #75, Kotlin in #77, and Go in
  #79) — next up per the loop is whichever `parity-gap` issue is oldest
  and unblocked (C#, per the filing order).

## PR #79 — Add Go language support
**2026-07-23** · [#79](https://github.com/baileyrd/rusty_repo_wise/pull/79) · closes [#31](https://github.com/baileyrd/rusty_repo_wise/issues/31)

- **Added:** a `repowise-parser` extractor for Go — structs, interfaces
  (mapped to `Trait`), functions, and methods. Go has no nested class
  scoping (methods are top-level declarations carrying a receiver
  clause, never nested inside the struct itself), so unlike every other
  language done so far, a method's `parent` is read directly from its
  receiver's type name rather than tracked via a scope stack. Import
  paths are resolved via a new `go_module_path` convention anchored on
  the nearest `go.mod`'s `module` declaration, mirroring Rust's
  `Cargo.toml`-anchoring.
- **Known limitation, stated plainly:** Go packages are directories
  (every file in one shares an import path), but the module-path index
  is one-file-per-path — a multi-file package only keeps the
  last-processed file as its resolved import target. Import edges still
  land in the right package, just not necessarily the exact file a
  symbol is defined in.
- 6 new tests (struct/interface/method extraction with receiver-based
  parent resolution, plain/aliased imports, selector/bare call tracking,
  cyclomatic complexity, duplicate-body hashing, interface-method-
  signature handling) plus a `repowise-graph` end-to-end test proving
  cross-package resolution via a real `go.mod`; 65 tests passing
  workspace-wide. Fourth language merged out of this session's
  `parity-loop` gap-analysis pass (after TypeScript/JavaScript in #26,
  Java in #75, and Kotlin in #77) — next up per the loop is whichever
  `parity-gap` issue is oldest and unblocked (C++, per the filing order).

## PR #77 — Add Kotlin language support
**2026-07-23** · [#77](https://github.com/baileyrd/rusty_repo_wise/pull/77) · closes [#30](https://github.com/baileyrd/rusty_repo_wise/issues/30)

- **Added:** a `repowise-parser` extractor for Kotlin — classes,
  interfaces (mapped to `Trait`), objects, and functions/methods.
  `repowise-graph`'s Java-only `java_module_path` was generalized to
  `jvm_module_path`, now recognizing both `src/main/java`/`src/test/java`
  and `src/main/kotlin`/`src/test/kotlin` source roots with both
  languages sharing one module-path index, so a mixed Java/Kotlin project
  resolves imports across both. Kotlin has no `new` keyword, so class
  instantiation (`Widget()`) is already covered by ordinary
  call-expression handling — no separate node-kind handler needed, unlike
  Java/TS/JS.
- **Known limitation, stated plainly:** secondary constructors aren't
  extracted as symbols (only the primary constructor's parameters,
  captured implicitly as part of the class symbol's span) — a narrower
  scope than Java's explicit constructor-declaration handling, accepted
  to keep this PR's scope reasonable.
- 6 new tests (class/interface/object/method extraction, plain/aliased/
  wildcard imports, bare-invocation-as-class-call tracking, cyclomatic
  complexity, duplicate-body hashing, interface-method-signature
  handling) plus a `repowise-graph` end-to-end test proving cross-language
  resolution (a Kotlin file importing a Java class in the same project);
  59 tests passing workspace-wide. Third language merged out of this
  session's `parity-loop` gap-analysis pass (after TypeScript/JavaScript
  in #26 and Java in #75) — next up per the loop is whichever
  `parity-gap` issue is oldest and unblocked (Go, per the filing order).

## PR #75 — Add Java language support
**2026-07-23** · [#75](https://github.com/baileyrd/rusty_repo_wise/pull/75) · closes [#29](https://github.com/baileyrd/rusty_repo_wise/issues/29)

- **Added:** a `repowise-parser` extractor for Java — classes, interfaces
  (mapped to `Trait`), enums, records, methods, and constructors (recorded
  as methods). Interface method signatures with no body are still
  recorded as symbols (0 complexity), same treatment as Rust's
  trait-method signatures. `import`/`import static`/wildcard imports are
  resolved via a new `java_module_path` convention anchored on the
  conventional Maven/Gradle `src/main/java`/`src/test/java` source root
  when present (falling back to repo-root-relative otherwise, same
  heuristic tradeoff as Python's dotted-path resolution). `new Type(...)`
  is recorded as a call to the constructed class, matching TS/JS's
  `new_expression` treatment, so instantiated classes don't read as dead
  code.
- **Known limitation, stated plainly:** no classpath/JAR-dependency
  resolution — bare (non-source-tree) references are left unresolved,
  same tradeoff already made for npm packages and Cargo dependencies. A
  nonstandard source layout (not `src/main/java`-anchored) falls back to
  a repo-root-relative package path, which may not match the file's real
  package declaration.
- 6 new tests (class/interface/method/constructor extraction, plain/
  static/wildcard imports, `new`-expression call tracking, cyclomatic
  complexity, duplicate-body hashing, interface-method-signature
  handling) plus a `repowise-graph` end-to-end Maven-layout resolution
  test; 52 tests passing workspace-wide. Second language merged out of
  this session's `parity-loop` gap-analysis pass (after TypeScript/
  JavaScript in #26, and hotspot scoring in #73) — next up per the loop
  is whichever `parity-gap` issue is oldest and unblocked.

## PR #73 — Add recency-weighted hotspot scoring
**2026-07-23** · [#73](https://github.com/baileyrd/rusty_repo_wise/pull/73) · closes [#28](https://github.com/baileyrd/rusty_repo_wise/issues/28)

- **Added:** `repowise-git`'s `hotspots()` now ranks files by a
  recency-weighted variant of churn × complexity — each commit
  contributes `exp(-age_days / 90)` toward a file's "decayed churn"
  instead of a flat `1`, so a file touched recently outranks an
  equally-churny file that's gone quiet. `CommitInfo` gained an
  author-date timestamp field to support this. The existing raw
  `score`/`churn` fields are unchanged and still reported (CLI, dashboard)
  alongside the new `decayed_score`.
- **Known limitation, stated plainly:** the decay half-life (90 days) is a
  fixed, documented constant, not configurable — no per-repo tuning.
  The bug-fix heuristic (message keywords only, no linked-issue-reference
  signal) remains a separate, still-open gap (issue #60).
- 1 new test (verifying both the decayed-score values and the resulting
  rank order — a recently-touched file outranks an equally-churny old
  one); 45 tests passing workspace-wide. First PR merged out of this
  session's `parity-loop` gap-analysis pass against the real repowise
  (45 gap issues filed: #28, #29-40 languages, #41-45 MCP tools, #46-50
  ADR sources, #51-56 health markers, #57-59 dashboard, #60 git analytics,
  plus 13 `needs-human` issues for product-direction/design questions
  outside the loop's auto-implement scope).

## PR #26 — Add TypeScript/JavaScript language support
**2026-07-23** · [#26](https://github.com/baileyrd/rusty_repo_wise/pull/26) · closes [#22](https://github.com/baileyrd/rusty_repo_wise/issues/22)

- **Added:** a `repowise-parser` extractor for TypeScript and JavaScript —
  functions, methods, classes, TypeScript interfaces (mapped to `Trait`),
  and named arrow-function/function-expression bindings, each with
  cyclomatic complexity, parameter count, and a duplicate-code body hash.
  ESM `import`/CommonJS `require` are resolved directly against the
  filesystem for relative (`./`, `../`) specifiers, same as Rust's
  `mod foo;`. `new ClassName(...)` is recorded as a call to the class so
  instantiated classes don't read as dead code. Bumped `tree-sitter` and
  the Rust/Python grammar crates to `LanguageFn`-based versions so
  `tree-sitter-typescript`/`tree-sitter-javascript` share the same core.
- **Known limitation, stated plainly:** no `node_modules` resolution
  (bare/npm specifiers are left unresolved) and no `tsconfig.json` path
  alias support — both explicitly out of scope per issue #22. A `new
  ClassName()` call resolves to the class, not the constructor method
  itself, so an only-ever-`new`'d class's constructor can still read as
  possibly-dead-code — a known heuristic gap, not a bug.
- 9 new tests (symbol/class/interface extraction, arrow/function-expression
  bindings, ESM+CommonJS imports, `new`-expression call tracking, cyclomatic
  complexity, duplicate-body hashing) plus a `repowise-graph` end-to-end
  relative-import-resolution test; 44 tests passing workspace-wide. Also
  filed issues #23 (MCP `get_risk`/`get_change_risk`), #24 (dashboard live
  search/drill-down/auto-refresh), and #25 (ADR mining additional sources)
  covering the other previously-called-out known limitations.

## PR #20 — Add static-site dashboard
**2026-07-23** · [#20](https://github.com/baileyrd/rusty_repo_wise/pull/20) · closes [#10](https://github.com/baileyrd/rusty_repo_wise/issues/10)

- **Added:** a new `repowise-dashboard` crate rendering a single
  self-contained HTML page — overview, code health, hotspots, and
  architectural decisions — from data the other layers already compute.
  No server, no JS build step, no templating engine; regenerate with
  `repowise dashboard [PATH]` and open `.repowise/dashboard/index.html`
  in a browser. All user-controlled text (language labels, file paths,
  decision titles) is HTML-escaped.
- **Known limitation, stated plainly:** static only — no live search,
  no per-file drill-down, no auto-refresh; re-run the command after the
  repo changes. Git-history and ADR/decision data degrade gracefully to
  explicit "not available" placeholders when a root has no git history
  or no ADRs, rather than failing the whole generation.
- 4 new tests (escaping, relative-path rendering with placeholders,
  hotspots/decisions rendering, an end-to-end real-index integration
  test); 36 tests passing workspace-wide. With this, all five of
  repowise's original "intelligence layers," its MCP server, and a
  dashboard have at least partial implementations in this port.

## PR #18 — Add MCP server: get_overview, search_codebase, get_context
**2026-07-22** · [#18](https://github.com/baileyrd/rusty_repo_wise/pull/18) · closes [#9](https://github.com/baileyrd/rusty_repo_wise/issues/9)

- **Added:** a new `repowise-mcp` crate exposing `get_overview`,
  `search_codebase`, and `get_context` as MCP tools over stdio via the
  official `rmcp` SDK, wired up as `repowise serve [PATH]`. `get_context`
  bundles a file's symbols, resolved deps/dependents, and health
  score/findings into one call — the tool that matters most for the
  original's "cut agent token spend on context-loading" goal.
- **Known limitation, stated plainly:** `get_risk`/`get_change_risk` are
  deferred to a follow-up rather than bundled in — they'd read naturally
  on `repowise-git`'s hotspot data. No caching across tool calls (same
  choice already made for `hotspots`/`ownership`/`coupled`/`decisions`).
- Verified the `rmcp` API against the actual installed crate's own
  doctests before writing real code — a fetched README described an
  older major version that didn't match what `cargo add` resolves.
- 5 new tests calling each tool method against a real index built by the
  actual indexing pipeline; 32 tests passing workspace-wide. With this,
  all five of repowise's original "intelligence layers" plus its MCP
  server have at least partial implementations in this port — only the
  web dashboard remains unstarted.

## PR #16 — Add architectural-decision (ADR) mining layer
**2026-07-22** · [#16](https://github.com/baileyrd/rusty_repo_wise/pull/16) · closes [#8](https://github.com/baileyrd/rusty_repo_wise/issues/8)

- **Added:** a new `repowise-adr` crate mining decisions from `docs/adr/*.md`
  files and decision-like commit messages (keyword heuristic), linking each
  to the files/symbols its body mentions, and tracking supersession via an
  ADR's existing `Status: Superseded by ADR-XXXX` line (no new front-matter
  convention needed). Wired up as `repowise decisions [PATH] [--for-file <FILE>]`.
  `repowise-git` gained `collect_commits()` so this reuses its git-log
  parsing instead of duplicating it.
- **Known limitation, stated plainly:** only 2 of the original repowise's 8
  decision sources are implemented (ADR files, commit messages) — PR
  descriptions, code comments, and integrations this repo doesn't have
  (Slack, issue trackers) are not mined. Linking is text matching, not
  semantic — a decision that only refers to a file descriptively won't
  be linked.
- 6 new tests (ADR parsing, unfilled-template skip, decision-commit
  detection, file/symbol linking, an end-to-end real-git-repo test); 27
  tests passing workspace-wide. With this, all five of repowise's
  original "intelligence layers" have CLI-facing implementations in this
  port (each covering a subset of the original's scope per layer).

## PR #14 — Add auto-generated documentation layer: per-file wiki pages
**2026-07-22** · [#14](https://github.com/baileyrd/rusty_repo_wise/pull/14) · closes [#7](https://github.com/baileyrd/rusty_repo_wise/issues/7)

- **Added:** a new `repowise-docs` crate rendering one deterministic
  markdown page per indexed file under `.repowise/wiki/`: symbol list,
  resolved dependencies/dependents (`repowise-graph`), and health
  findings (`repowise-health`). No LLM involved. Wired up as
  `repowise docs [PATH]`.
- **Known limitation, stated plainly:** freshness (new/changed/unchanged)
  is tracked via a hash of each file's *own* source only — a page can
  report "unchanged" while its rendered content actually differs, if
  what changed was cross-file data (a new caller, a health finding from
  another file). Pages are always rewritten with current data regardless
  of the reported status, so content itself is never stale, only the
  status label can undersell how much changed.
- 2 new tests (a `render_page` unit test, a real-directory integration
  test for the New/Changed/Unchanged transitions); 21 tests passing
  workspace-wide.

## PR #12 — Add git-analytics layer: churn, hotspots, ownership, co-change coupling
**2026-07-22** · [#12](https://github.com/baileyrd/rusty_repo_wise/pull/12) · closes [#6](https://github.com/baileyrd/rusty_repo_wise/issues/6)

- **Added:** a new `repowise-git` crate computing git-history analytics by
  shelling out to `git log`/`git blame` — per-file churn, hotspot score
  (churn × total cyclomatic complexity, reusing `repowise-parser`'s
  existing complexity data), bug-fix-commit frequency (message-keyword
  heuristic), co-change coupling, and per-author line ownership. Wired
  up as `repowise hotspots`/`ownership`/`coupled`.
- **Known limitation, stated plainly:** git analytics are computed fresh
  on every invocation rather than cached in `.repowise/index.json`, to
  avoid taking on cache-invalidation design in this pass. Bug-fix
  detection is a message-keyword heuristic, not linked-issue-aware.
- 4 new integration tests build real, disposable git repos (via the
  `git` CLI) to exercise this against actual `git log`/`git blame`
  output; 19 tests passing workspace-wide.

## PR #4 — Update default-branch references now that main exists
**2026-07-22** · [#4](https://github.com/baileyrd/rusty_repo_wise/pull/4)

- **Changed:** the repo's default branch was renamed on GitHub from
  `claude/repowise-rust-port-pcxhal` to `main`. Updated the two places that
  hardcoded the old name: `CLAUDE.md`'s workflow description and
  `ci-rust.yml`'s `push` trigger (previously pinned to the old branch name
  with a comment to update it once `main` existed).
- Earlier entries in this file that mention the old branch name describe
  the state at the time those changes were made and are left as an
  accurate historical record rather than rewritten.

## PR #1 — Add standard governance files (PR/issue templates, docs, CI)
**2026-07-22** · [#1](https://github.com/baileyrd/rusty_repo_wise/pull/1)

- **Added:** the standard governance-file set — PR/issue templates,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, this file, ARCHITECTURE
  (hand-adapted to this repo's actual crate layout), an ADR seed, and a Rust
  CI workflow (fmt + clippy + test) gating merges going forward.
- **Fixed:** three pre-existing `clippy::unnecessary_sort_by` lints (two in
  `repowise-graph`, one in `repowise-health`) that the new CI caught — this
  repo had never run clippy in CI before, so they'd gone unnoticed locally
  under an older clippy version.
- **Known limitation, stated plainly:** the repo's GitHub Actions "allowed
  actions" policy initially blocked `actions/checkout`/`actions/cache`
  entirely (first-party actions), which had to be fixed in repo settings
  before CI could run at all — not something a workflow-file change alone
  could fix. Also: `ci-rust.yml` triggers on pushes to
  `claude/repowise-rust-port-pcxhal` specifically since there's no `main`
  yet; update that trigger once a conventional default branch exists.

## 2026-07-22 — Add deterministic code-health scoring layer
[`088dad1`](https://github.com/baileyrd/rusty_repo_wise/commit/088dad137b8cca871f1aeaf671a46e6776e81b35)

- **Added:** a new `repowise-health` crate scoring every indexed file 0–10 from
  six deterministic markers — long functions, high cyclomatic complexity,
  oversized parameter lists, god classes, duplicate code, and possibly-dead
  code — wired up as `repowise health [PATH]`. `repowise-parser` now computes
  per-function cyclomatic complexity, parameter count, and a duplicate-code
  body hash; `repowise-graph` gained `call_in_degree()` to support the
  dead-code check.
- **Known limitation, stated plainly:** covers 6 of the original repowise's
  ~25 health markers. Git-history-based markers (churn/hotspots, ownership,
  co-change coupling), LCOM4 cohesion, and substring-level (Rabin-Karp) clone
  detection are deferred — this only detects whole-function-body duplicates
  via exact hash match, not partial/near-duplicate code.
- 9 new tests (5 health-scoring, 4 parser); 15 tests passing workspace-wide.

## 2026-07-22 — Scaffold Rust port of repowise: dependency-graph layer + CLI
[`1d45806`](https://github.com/baileyrd/rusty_repo_wise/commit/1d458060e72fc33b001cf8800a57d0e90d35874c)

- **Added:** initial Rust workspace (`repowise-core`, `repowise-parser`,
  `repowise-graph`, `repowise-cli`) implementing the dependency-graph
  intelligence layer from repowise: tree-sitter extraction of symbols,
  imports, and calls for Rust and Python, a petgraph-backed dependency graph
  with directory-layout-based import/call resolution, and
  `init`/`update`/`overview`/`search`/`deps` CLI commands.
- **Known limitation, stated plainly:** import/call resolution is heuristic
  (directory-layout conventions), not full compiler-grade name resolution —
  ambiguous or external references are left unresolved rather than guessed.
  Only Rust and Python are parsed; repowise's other 14 languages aren't
  implemented. Git analytics, doc generation, ADR mining, the MCP server, and
  the web dashboard are out of scope for now.
- 6 tests passing (2 graph integration tests, 4 parser unit tests).
