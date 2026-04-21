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
swift-api-tool <package-path> -o public-api.yaml
```

Output format is inferred from the extension (`.md`, `.yaml`/`.yml`,
`.html`/`.htm`), or forced with `--format md|yaml|html`.

### Examples

```sh
# Markdown: one big reference doc.
swift-api-tool . -o docs/public-api.md

# YAML: compact, nested, great for line-based diffs.
swift-api-tool . -o public-api.yaml

# HTML: self-contained browsable file with sidebar nav and filter.
swift-api-tool . -o public-api.html
```

## What's included

- Every Swift library target exposed by a `library` product.
- All `public` (and `open`) symbols: types, protocols, functions, methods,
  properties, subscripts, enum cases, typealiases, associated types.
- Attributes (`@MainActor`, `@propertyWrapper`, `@available`, etc.) are
  preserved in declarations.
- Cross-module extensions are grouped under a synthesized
  `extension <Type>` node.

## What's not included

- Doc comments.
- Symbols below `public` (use `--min-access-level` — not yet implemented
  for non-public levels).
- Same-module extensions as distinct groups (Swift merges these into the
  parent type unless `-emit-extension-block-symbols` is used).

## Using in CI for API-change detection

Commit a `public-api.yaml` snapshot to your repo, then in CI:

```yaml
- run: cargo install swift-api-tool
- run: swift-api-tool . -o /tmp/public-api.yaml
- run: diff public-api.yaml /tmp/public-api.yaml
```

The `diff` fails the job if the committed snapshot is stale — prompting
you to either update it or explain the API change.

## License

MIT. See [LICENSE](LICENSE).
