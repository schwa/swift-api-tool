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
status: open
priority: medium
kind: feature
labels: effort:m
created: 2026-08-30T14:22:42Z
updated: 2026-08-30T14:29:14Z
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

---

## 4: Source-oriented Swift package table of contents is missing

+++
status: open
priority: medium
kind: feature
labels: effort:l
created: 2026-08-30T14:22:42Z
updated: 2026-08-30T14:29:14Z
+++

The app cannot generate a source-oriented table of contents for a Swift package. The sibling `../swift-toc/` tool provides this capability but currently requires users to install and run a separate executable.

The missing output discovers Swift source files by target, parses their declarations, preserves source line numbers, sorts declarations by kind and name, and emits Markdown or JSON. It covers top-level protocols, structs, classes, actors, enums, extensions, type aliases, functions, variables, initializers, subscripts, and macros, including members declared in extensions and explicit access levels.

## Proposed fix (per user)

Merge the functionality from `../swift-toc/` into `swift-api-tool` and expose it through this app rather than retaining a separate tool.

- `2026-08-30T14:23:29Z`: Related to #5: both need source-oriented Swift declaration extraction and may share parsing infrastructure.

---

## 5: Documentation-comment Markdown generation is missing

+++
status: open
priority: medium
kind: feature
labels: effort:xl
created: 2026-08-30T14:22:56Z
updated: 2026-08-30T14:29:14Z
+++

The app cannot generate API documentation from Swift documentation comments. The sibling `../HeaderDocToMarkdown/` tool provides this capability but currently requires users to install and run a separate executable.

The missing behavior includes:

- Extracting `///` comments and declaration signatures from Swift package sources.
- Generating one Markdown document per target or exporting the extracted model as JSON.
- Grouping types and members by declaration kind.
- Merging extension members into types in the same target.
- Preserving conformances, where-clause constraints, access levels, and source locations.
- Filtering to public API by default, with an option to include non-public declarations.
- Reporting declarations that lack documentation.

## Proposed fix (per user)

Merge the functionality from `../HeaderDocToMarkdown/` into `swift-api-tool` and expose it through this app rather than retaining a separate tool.

- `2026-08-30T14:23:29Z`: Related to #4: both need source-oriented Swift declaration extraction and may share parsing infrastructure.

---
