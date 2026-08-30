# ISSUES.md

File format: <https://github.com/schwa/issues-format>

---

## 1: Built-in diff for nicer CI output

+++
status: closed
priority: none
kind: feature
created: 2024-04-21T00:00:00Z
updated: 2026-04-22T01:11:21Z
closed: 2026-04-22T01:11:21Z
+++

Currently, users comparing a committed API snapshot against a freshly
generated one rely on `diff -u`. The output is a raw unified diff of the
YAML, which shows textual line changes but doesn't convey *semantic*
changes well:

- Whitespace / block-scalar indentation changes look identical to real
  API changes.
- A modifier-only change (e.g. adding `@Sendable` to a closure parameter)
  appears as a pair of `-` / `+` lines with no visual highlight of what
  actually moved.
- Nesting context is lost: you can't easily tell which type a changed
  member belongs to unless you scroll up through the diff.
- Renaming a symbol looks like an unrelated "removed X / added Y" pair.

### Proposal

Add a `swift-api-tool diff <old> <new>` subcommand (or `--check <file>`
on the main command) that:

1. Parses both YAML snapshots into the `PackageModel` tree.
2. Walks both trees together, pairing symbols by a stable key (start
   with "full path derived from decl"; later, optionally USR if we
   decide to include it).
3. Classifies each difference:
   - **Added** — in new, not in old
   - **Removed** — in old, not in new
   - **Changed** — same key, different `decl`
4. Emits a concise, grouped report with ANSI color by default (TTY
   detection) and a `--no-color` flag, plus a `--format markdown` mode
   for CI logs / PR comments. Example:
   ```
   MetalSprockets
     Element
       ~ onCommandBufferScheduled(_:) — @Sendable closure dropped
       - shaderScope(_:)
     - ShaderScope
   ```
5. Exits non-zero when there are differences (so it can still drive CI).

### Open questions

- Pairing key: `decl` text is brittle across whitespace/ordering changes
  but is what we have today. Including the USR in the YAML would make
  this bulletproof (at the cost of one more line per symbol).
- Should the diff renderer live in this crate, or a separate
  `swift-api-diff` binary?
- Worth emitting SARIF or GitHub PR annotations so changes show up inline
  on the PR diff view?

- `2026-04-22T01:11:21Z`: Implemented via new 'swift-api-tool diff' subcommand with --allow-additive mode.

---

## 2: `--allow-additive` mode for CI drift checks

+++
status: closed
priority: none
kind: feature
depends: 1
created: 2024-04-21T00:00:00Z
updated: 2026-04-22T01:11:21Z
closed: 2026-04-22T01:11:21Z
+++

When using the tool in CI as an API-drift gate, some projects want to
block only *breaking* changes (removals, signature changes) while
letting *additive* changes (new public symbols) pass without a snapshot
update. A raw `diff -u` can't distinguish these.

This depends on the proposed built-in `diff` subcommand (see issue 1).

### Proposal

Once `swift-api-tool diff <old> <new>` exists, add a `--allow-additive`
flag (alternate names: `--additive-ok`, `--breaking-only`) that changes
the exit code semantics:

- Exit 0 if *only* additions are present.
- Exit non-zero if any removals or signature changes are found.
- Still print all three categories in the report.

### Open questions

- What counts as "additive"? Proposed default:
  - **Additive**: a new symbol appears.
  - **Breaking**: a symbol disappears, or its `decl` changes in any
    non-cosmetic way.
- Is adding a protocol requirement additive or breaking? (Breaking for
  conformers; additive for callers.) Likely: treat any `decl` change on
  an existing symbol as breaking, regardless of category.
- Should we also offer `--allow-cosmetic` for things like reordered
  attributes or whitespace? Probably not — the renderer should already
  be normalized, so cosmetic diffs should not exist.
- Adding a default value to a parameter is source-compatible but still
  changes the `decl` string. Do we try to detect that, or treat it as a
  plain breaking change and let the human override?

- `2026-04-22T01:11:21Z`: Implemented via new 'swift-api-tool diff' subcommand with --allow-additive mode.

---

## 3: Package dependency graph output is missing

+++
status: closed
priority: medium
kind: feature
labels: effort:m
created: 2026-08-30T14:22:42Z
updated: 2026-08-30T14:34:19Z
closed: 2026-08-30T14:34:19Z
+++

The app cannot generate a dependency graph for a Swift package. The sibling `../spm_to_graph/` tool provides this capability but currently requires users to install and run a separate executable.

The missing behavior includes:

- Reading package targets and their target and product dependencies.
- Optionally excluding test targets.
- Optionally excluding external product dependencies.
- Writing Graphviz DOT directly.
- Rendering SVG or PNG through Graphviz based on the requested output extension.

## Proposed fix (per user)

Merge the functionality from `../spm_to_graph/` into `swift-api-tool` and expose it through this app rather than retaining a separate tool.

- `2026-08-30T14:34:19Z`: Implemented the graph subcommand with DOT, SVG, and PNG output plus target and product dependency filters.

---

## 4: Source-oriented Swift package table of contents is missing

+++
status: closed
priority: medium
kind: feature
labels: effort:l
created: 2026-08-30T14:22:42Z
updated: 2026-08-30T15:13:24Z
closed: 2026-08-30T15:13:24Z
+++

The app cannot generate a source-oriented table of contents for a Swift package. This differs from the existing API snapshot: the snapshot is compiler-derived, public-API-oriented, and grouped by module/type, while this feature is grouped by target/source file and includes declaration line numbers and non-public declarations.

The reference implementation was the standalone Swift package formerly at `../swift-toc/`. It is being archived in `/Users/schwa/Shared/Projects/Project Graveyard.asif`; preserve or mount that archive when exact source behavior is needed. Its main implementation file was `Sources/swift-toc/swift_toc.swift`.

## Reference behavior

CLI:

```text
swift-toc [path] [-o|--output <file>] [-f|--format markdown|json]
```

Defaults: package path `.`, Markdown output, and stdout when no output path is supplied. When writing a file, print `TOC written to <path>`.

Package discovery:

1. Require `<path>/Package.swift`; report `Not a Swift package: <path> (Package.swift not found)` otherwise.
2. Use the package directory name as the package name.
3. Enumerate immediate subdirectories of `Sources` as targets, sorted by name.
4. Recursively enumerate non-hidden `.swift` files in each target.
5. Store each source path relative to the target root and sort files with localized standard ordering.
6. Omit targets and files that contain no recognized declarations.

Parsing used `SwiftParser.Parser.parse` and a source-accurate `SwiftSyntax.SyntaxVisitor`. Each declaration records kind, name, optional signature, optional explicit access level, one-based source line, and children. Recognized kinds, in sort order, are protocol, struct, class, actor, enum, extension, typealias, initializer, subscript, function, variable, constant, and macro. Sort declarations first by this kind order and then by localized-standard name.

At file scope, recognize protocols, structs, classes, actors, enums, extensions, type aliases, functions, variables/constants, initializers, subscripts, and macros. Do not descend into ordinary type declarations. For extensions only, collect direct functions, initializers, subscripts, variables/constants, type aliases, structs, classes, and enums as children; nested declarations are not recursively expanded.

Access-level extraction recognizes explicit `private`, `fileprivate`, `internal`, `package`, `public`, and `open` modifiers. A declaration with no access modifier stores no access level rather than inferring `internal`.

Signature extraction:

- Functions and initializers include argument labels, local parameter names where present, and parameter types.
- Failable initializers retain `?` or `!` before the parameter list.
- Subscripts use `[parameters] -> ReturnType`.
- Other declaration kinds display only kind and name.

Markdown output:

```text
# <package>

## <target>

### <relative source file>

- [<explicit access> ]<kind> <name/signature> // line <line>
  - <extension child>
```

JSON output is the Codable hierarchy `Package { name, targets }`, `Target { name, sourceFiles }`, `SourceFile { name, declarations }`, and recursive `Declaration { kind, name, signature?, accessLevel?, line, children }`, pretty-printed.

## Acceptance criteria

- A command in `swift-api-tool` reproduces the discovery, declaration coverage, ordering, line numbers, Markdown hierarchy, and JSON model above.
- Markdown and JSON can be written to a file; Markdown can be emitted to stdout.
- Tests cover declaration ordering, explicit access levels, function/init/subscript signatures, extension children, relative paths, empty targets, and both renderers.
- README documents how this source-oriented TOC differs from the existing public API snapshot.

## Proposed fix (per user)

Embed this capability in `swift-api-tool` rather than retaining a separate executable. A Rust-native Swift parser is preferred if this remains a Rust project. If the project migrates to Swift, port the archived SwiftSyntax implementation and tests directly.

This overlaps with #5. Share package discovery and source parsing if both features are implemented.

- `2026-08-30T14:23:29Z`: Related to #5: both need source-oriented Swift declaration extraction and may share parsing infrastructure.
- `2026-08-30T14:44:51Z`: Implementation note: tree-sitter-swift is a good fit for this source-oriented feature. Use the maintained alex-pinkus/tree-sitter-swift grammar from Rust to extract declaration kinds, names, explicit access modifiers, source line ranges, and extension children. Preserve signatures by slicing the original source using node byte ranges, then normalize whitespace, rather than rebuilding text from syntax nodes. Treat parse ERROR nodes defensively and add fixtures for attributes, macros, failable initializers, complex parameters, constrained extensions, and current Swift syntax. A shared Rust declaration-tree/JSON model can also support the source-only parts of #5.
- `2026-08-30T15:13:24Z`: Wontfix: doc output now carries path:line source locations for public symbols; remaining unique value (non-public declarations, per-file grouping) does not justify a second parsing pipeline. Full reproduction detail preserved in the description and the archived swift-toc source.

---

## 5: Documentation-comment Markdown generation is missing

+++
status: closed
priority: medium
kind: feature
labels: effort:xl
created: 2026-08-30T14:22:56Z
updated: 2026-08-30T15:07:31Z
closed: 2026-08-30T15:07:31Z
+++

The app does not attach Swift documentation comments to API declarations or generate documentation-focused Markdown. Existing symbol-graph output captures signatures and hierarchy, but its model and renderers discard documentation text and source locations.

The reference implementation was the standalone Swift package formerly at `../HeaderDocToMarkdown/`. It is archived in `/Users/schwa/Shared/Projects/Project Graveyard.asif`; preserve or mount that archive when exact behavior or tests are needed. Relevant archived files are:

- `Sources/HeaderDocToMarkdown/HeaderDocToMarkdown.swift`: CLI and per-target output.
- `Sources/HeaderDocToMarkdownLib/Documentation.swift`: Codable model.
- `Sources/HeaderDocToMarkdownLib/DocumentationExtractor.swift`: package/target/source discovery.
- `Sources/HeaderDocToMarkdownLib/DocumentationVisitor.swift`: SwiftSyntax extraction.
- `Sources/HeaderDocToMarkdownLib/MarkdownGenerator.swift`: grouping and rendering.
- `Tests/HeaderDocToMarkdownTests/`: model, visitor, and renderer tests.

The archived package uses Swift 6.2, swift-argument-parser, SwiftParser, and SwiftSyntax. It exposes both a `headerdoc2md` executable and `HeaderDocToMarkdownLib` library.

## Reference CLI behavior

```text
headerdoc2md [-p|--package-path <path>] [-o|--output <directory>]
             [-i|--include-private] [--json] [--verbose]
```

Defaults: package path `.`, output directory `.`, public/open declarations only, Markdown output, and non-verbose progress. Require `Package.swift`. Create the output directory. Discover non-test targets through `swift package dump-package`. Respect target custom paths and explicit source lists; otherwise use `Sources/<target>`. Recursively read sorted, non-hidden `.swift` files.

Generate one `<target>.md` or `<target>.json` file for each target containing declarations. Skip missing source directories and empty targets. Verbose mode reports package path, discovered targets, per-target progress, missing sources, and empty targets.

## Extraction model and behavior

Parse each file with `SwiftParser.Parser.parse` and a source-accurate `SwiftSyntax.SyntaxVisitor`.

Models:

```text
DocumentedType {
  name, kind, signature, documentation, file, members, conformances
}
DocumentedMember { name, kind, signature, documentation }
Conformance { protocolName, whereClause? }
```

All models used for JSON are Codable. Type kinds are struct, class, enum, protocol, actor, extension, typealias, function, and property. Member kinds are initializer, method, property, subscript, and enum case.

Documentation extraction supports both `///` and `/** ... */` leading trivia. Strip comment delimiters and leading/trailing horizontal whitespace, preserve line order, and use an empty string for undocumented declarations.

Recognize explicit `private`, `fileprivate`, `internal`, `public`, and `open`; declarations without a modifier are internal. The default minimum is public. `--include-private` lowers the minimum to private. Protocol and extension members inherit their parent’s effective visibility while explicit modifiers may restrict visibility. Preserve relevant declaration modifiers such as `final` in signatures.

Extract:

- Structs, classes, enums, protocols, actors, extensions, and type aliases.
- Top-level functions and stored/computed properties.
- Initializers, methods, properties, subscripts, and enum cases inside types/extensions.
- Inheritance and protocol conformances, including generic `where` clauses.
- Relative source file paths.

Normalize signatures to one line, omit bodies and comments, and collapse repeated whitespace. Extension names remove generic arguments for grouping. Omit extensions with no included members. Track internal protocols so they are not presented as public conformances.

Important reference limitation: the visitor stores a single `currentType`; nested types are not modeled robustly. Reproduce intended output, not accidental state corruption from nested declarations.

## Markdown output

Generate one document per target:

1. `# <target>`.
2. Type sections in this order: Protocols, Classes, Structures, Actors, Enumerations, Type Aliases.
3. External Extensions.
4. Top-level Functions.
5. Top-level Properties.

Sort declarations and most member groups by name. Merge members and conformances from same-target extensions into their base type. Keep extensions of external types in the Extensions section.

For each type, render its name, documentation, fenced Swift signature, public conformances with constraints, relative source path, and member sections in this order: Properties, Initializers, Methods, Subscripts, Cases. Render a warning block for an undocumented declaration by default:

```markdown
> ⚠️ **Missing documentation.**
```

JSON output is a pretty-printed, sorted-key array of `DocumentedType` values per target.

## Preferred integration with current architecture

The current app already consumes `swift package dump-symbol-graph`. Symbol graph entries can provide documentation comments and source locations for compiler-visible API, so the first implementation should extend the existing `Symbol`, `SymbolNode`, and output models rather than introduce a second source parser solely for public API documentation.

A focused initial scope is:

- Decode documentation comments and source locations from symbol graph JSON.
- Preserve them in YAML so snapshots round-trip without data loss.
- Render documentation in Markdown and HTML.
- Add an opt-in report or warning for undocumented public symbols.

Source parsing remains necessary only if exact parity with `--include-private`, per-source discovery, or the archived renderer is required. If #4 is also implemented, share its source parser rather than adding another one.

## Acceptance criteria

- Public symbol documentation and source locations survive extraction and serialization.
- Markdown and HTML associate documentation with the correct declaration and preserve Markdown content.
- Extension members retain their documentation after grouping.
- Undocumented-symbol reporting is configurable and does not alter normal exit status unless explicitly requested.
- Tests cover line/block comments, multiline Markdown, undocumented symbols, overloads, extension members, constrained conformances, source locations, and YAML backward compatibility when documentation fields are absent.
- README explains documentation output and the boundary between compiler-visible public API and optional private/source parsing.

## Proposed fix (per user)

Embed the useful behavior from archived `HeaderDocToMarkdown` in `swift-api-tool`. Prefer extending the existing symbol-graph pipeline for public API documentation. Port the archived SwiftSyntax visitor only if non-public declaration support remains a requirement.

- `2026-08-30T14:23:29Z`: Related to #4: both need source-oriented Swift declaration extraction and may share parsing infrastructure.
- `2026-08-30T14:44:51Z`: Implementation note: use a hybrid approach. Keep symbol graphs authoritative for compiler-visible public/open API documentation and source locations; extend the existing decoder and output models first. Use tree-sitter-swift only for source-oriented behavior that symbol graphs do not cover, especially --include-private, per-file grouping, and leading /// or /** */ comment association. Share the Rust declaration-tree/JSON layer proposed for #4. Tree-sitter cannot provide compiler-resolved visibility, inferred types, or conformances, so those should not replace symbol-graph data. Add fixtures for trivia attachment, attributes, macros, constrained extensions, overloads, and ERROR-node recovery.
- `2026-08-30T15:07:31Z`: Implemented via symbol-graph pipeline: doc comments and source locations decoded, preserved in YAML, rendered in Markdown/HTML, plus --report-undocumented.

---

## 6: Single-file output does not scale to large packages

+++
status: new
priority: medium
kind: enhancement
created: 2026-08-30T15:07:37Z
+++

All output formats write one file. For a large package (e.g. MetalSprockets, ~7,300 lines of Markdown) a single document is hard to navigate, review, and link into.

There is no way to write output as a directory hierarchy, such as one file per module, per type, or per source-file grouping, with an index file linking the pieces. This affects Markdown most, but YAML snapshots and HTML would also benefit from a split layout for very large APIs.

---
