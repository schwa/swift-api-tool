# swift-api-tool

Extract the public API surface of a Swift Package into a single file —
Markdown, YAML, or HTML — suitable for review, documentation, or diffing
in CI.

Internally it drives `swift package dump-symbol-graph` and walks the emitted
symbol graphs: one per library target, plus per-external-module extension
graphs.

## Install

```sh
cargo install swift-api-tool
```

Requires the Swift toolchain (`swift` on `$PATH`).

## Usage

```sh
# YAML API snapshot (the default subcommand):
swift-api-tool api <package-path> -o public-api.yaml
swift-api-tool <package-path>              # same thing

# Documentation as Markdown or HTML:
swift-api-tool doc <package-path> -o public-api.md
swift-api-tool doc <package-path> -o public-api.html
```

`api` always writes YAML. `doc` infers Markdown or HTML from the output
extension (`.md`, `.html`/`.htm`), or takes `--format md|html`.

### Examples

```sh
# YAML: compact, nested, great for line-based diffs in CI.
swift-api-tool api .

# Markdown: one big reference doc with doc comments inline.
swift-api-tool doc . -o docs/public-api.md

# HTML: self-contained browsable file with sidebar nav and filter.
swift-api-tool doc . -o public-api.html

# Multi-file Markdown mirroring the package's source tree:
# docs/index.md plus docs/Sources/<Module>/.../<File>.md per source file.
swift-api-tool doc . --split -o docs/
```

### Package dependency graphs

Generate a Graphviz dependency graph from `swift package describe`:

```sh
swift-api-tool graph . -o package.dot
swift-api-tool graph . -o package.svg --skip-test-targets
```

The output extension selects DOT, SVG, or PNG. SVG and PNG output require Graphviz's `dot` command. Use `--skip-product-dependencies` to omit external package products.

## What's included

- Every Swift library target exposed by a `library` product.
- All `public` (and `open`) symbols: types, protocols, functions, methods,
  properties, subscripts, enum cases, typealiases, associated types.
- Attributes (`@MainActor`, `@propertyWrapper`, `@available`, etc.) are
  preserved in declarations.
- Cross-module extensions are grouped under a synthesized
  `extension <Type>` node.
- Doc comments (`///` and `/** */`) and package-relative `path:line` source
  locations, attached to each symbol in every output format.

Pass `--report-undocumented` to also list public symbols without doc comments
on stderr (the exit status is unaffected).

## What's not included

- Symbols below `public` (use `--min-access-level` — not yet implemented
  for non-public levels).
- Same-module extensions as distinct groups (Swift merges these into the
  parent type unless `-emit-extension-block-symbols` is used).

## Using in CI for API-change detection

Commit a `public-api.yaml` snapshot to your repo, then in CI:

```yaml
- run: cargo install swift-api-tool
- run: swift-api-tool api . -o /tmp/public-api.yaml
- run: swift-api-tool diff public-api.yaml /tmp/public-api.yaml
```

The `diff` subcommand parses both snapshots semantically and prints a
colorized, grouped report of **added / removed / changed** symbols.
It exits non-zero when there are differences.
Doc-comment and source-location changes never count as API differences,
and snapshots created before those fields existed still diff cleanly.

Options:

- `--format markdown` — emit a Markdown report (good for PR comments).
- `--no-color` / `--color` — override the default TTY color detection.
- `--allow-additive` — exit 0 if the only changes are *additions*;
  exit non-zero only for removals or signature changes. Useful when you
  want CI to block breaking changes but allow new public API to land
  without a snapshot update.

For a plain textual unified diff, `diff -u` still works.

## License

MIT. See [LICENSE](LICENSE).
