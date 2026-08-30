//! Markdown rendering: single document and split (multi-file) output.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::model::{PackageModel, SymbolNode};

// --- Markdown rendering ---

pub(crate) fn render_md(model: &PackageModel) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", model.package));
    out.push_str(&format!(
        "_Public API surface. Access level: `{}`._\n\n",
        model.access_level
    ));
    for m in &model.modules {
        out.push_str(&format!("## Module `{}`\n\n", m.name));
        if m.symbols.is_empty() && m.extensions.is_empty() {
            out.push_str("_No symbol graph emitted._\n\n");
            continue;
        }
        if m.symbols.is_empty() {
            out.push_str("_No own symbols._\n\n");
        }
        for s in &m.symbols {
            render_md_symbol(s, 3, &mut out);
        }
        for ext in &m.extensions {
            out.push_str(&format!("### Extensions to `{}`\n\n", ext.extended_module));
            for s in &ext.symbols {
                render_md_symbol(s, 4, &mut out);
            }
        }
    }
    out
}

fn render_md_symbol(sym: &SymbolNode, depth: usize, out: &mut String) {
    render_md_decl(
        &sym.decl,
        sym.doc.as_deref(),
        sym.source.as_deref(),
        depth,
        out,
    );
    for child in &sym.members {
        render_md_symbol(child, depth + 1, out);
    }
}

fn render_md_decl(
    decl: &str,
    doc: Option<&str>,
    source: Option<&str>,
    depth: usize,
    out: &mut String,
) {
    let heading = "#".repeat(depth.min(6));
    // Use first line of declaration as the heading subject.
    let first_line = decl.lines().next().unwrap_or("");
    out.push_str(&format!("{heading} `{}`\n\n", first_line));
    out.push_str("```swift\n");
    out.push_str(decl);
    if !decl.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n\n");
    if let Some(doc) = doc {
        out.push_str(doc);
        out.push_str("\n\n");
    }
    if let Some(source) = source {
        out.push_str(&format!("<sub>Defined at `{}`</sub>\n\n", source));
    }
}

// --- Split (multi-file) Markdown rendering ---

/// One page per Swift source file, mirroring the package's source tree,
/// plus an index.md linking everything.
/// A page entry: either a real symbol or a synthesized extension wrapper
/// restricted to the members declared in one file.
enum PageItem<'a> {
    Symbol(&'a SymbolNode),
    Wrapper {
        decl: &'a str,
        members: Vec<&'a SymbolNode>,
    },
}

fn render_page_item(item: &PageItem, depth: usize, out: &mut String) {
    match item {
        PageItem::Symbol(symbol) => render_md_symbol(symbol, depth, out),
        PageItem::Wrapper { decl, members } => {
            render_md_decl(decl, None, None, depth, out);
            for member in members {
                render_md_symbol(member, depth + 1, out);
            }
        }
    }
}

pub(crate) fn render_md_split(model: &PackageModel) -> Vec<(PathBuf, String)> {
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut index = String::new();
    index.push_str(&format!("# {}\n\n", model.package));
    index.push_str(&format!(
        "_Public API surface. Access level: `{}`._\n\n",
        model.access_level
    ));

    for module in &model.modules {
        let mut by_file: BTreeMap<String, Vec<PageItem>> = BTreeMap::new();
        let mut unplaced: Vec<PageItem> = Vec::new();

        for symbol in &module.symbols {
            match source_file(symbol) {
                Some(file) => by_file
                    .entry(file)
                    .or_default()
                    .push(PageItem::Symbol(symbol)),
                None => unplaced.push(PageItem::Symbol(symbol)),
            }
        }

        // Extension-block wrappers have no source of their own; distribute
        // their members to the files where the members are declared.
        for ext in &module.extensions {
            for wrapper in &ext.symbols {
                let mut member_files: BTreeMap<String, Vec<&SymbolNode>> = BTreeMap::new();
                let mut member_unplaced: Vec<&SymbolNode> = Vec::new();
                for member in &wrapper.members {
                    match source_file(member) {
                        Some(file) => member_files.entry(file).or_default().push(member),
                        None => member_unplaced.push(member),
                    }
                }
                if wrapper.members.is_empty() || !member_unplaced.is_empty() {
                    unplaced.push(PageItem::Wrapper {
                        decl: &wrapper.decl,
                        members: member_unplaced,
                    });
                }
                for (file, members) in member_files {
                    by_file.entry(file).or_default().push(PageItem::Wrapper {
                        decl: &wrapper.decl,
                        members,
                    });
                }
            }
        }

        index.push_str(&format!("## Module `{}`\n\n", module.name));
        if by_file.is_empty() && unplaced.is_empty() {
            index.push_str("_No symbol graph emitted._\n\n");
            continue;
        }
        for (file, items) in &by_file {
            let page_path = md_page_path(file);
            index.push_str(&format!("- [{}]({})\n", file, page_path.display()));

            let mut page = String::new();
            page.push_str(&format!("# {}\n\n", file));
            page.push_str(&format!(
                "_Module `{}` — package `{}`._\n\n",
                module.name, model.package
            ));
            for item in items {
                render_page_item(item, 2, &mut page);
            }
            files.push((page_path, page));
        }
        if !by_file.is_empty() {
            index.push('\n');
        }
        for item in &unplaced {
            render_page_item(item, 3, &mut index);
        }
    }

    files.insert(0, (PathBuf::from("index.md"), index));
    files
}

/// Relative source file path of a symbol, without the `:line` suffix.
/// Absolute or traversal-prone paths are rejected so pages stay inside
/// the output directory.
fn source_file(symbol: &SymbolNode) -> Option<String> {
    let source = symbol.source.as_deref()?;
    let path = source.rsplit_once(':').map_or(source, |(p, _)| p);
    if path.starts_with('/') || path.split('/').any(|c| c == "..") {
        return None;
    }
    Some(path.to_string())
}

fn md_page_path(source_file: &str) -> PathBuf {
    let mut path = PathBuf::from(source_file);
    path.set_extension("md");
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ExtensionGroup, ModuleModel};

    #[test]
    fn markdown_renders_doc_and_source() {
        let node = SymbolNode {
            decl: "public struct S".to_string(),
            doc: Some("A thing.".to_string()),
            source: Some("Sources/M/S.swift:10".to_string()),
            members: vec![],
        };
        let mut out = String::new();
        render_md_symbol(&node, 3, &mut out);
        assert!(out.contains("A thing.\n\n"));
        assert!(out.contains("<sub>Defined at `Sources/M/S.swift:10`</sub>"));
    }

    fn node(decl: &str, source: Option<&str>, members: Vec<SymbolNode>) -> SymbolNode {
        SymbolNode {
            decl: decl.to_string(),
            doc: None,
            source: source.map(str::to_string),
            members,
        }
    }

    fn split_model() -> PackageModel {
        PackageModel {
            package: "P".to_string(),
            access_level: "public".to_string(),
            modules: vec![ModuleModel {
                name: "M".to_string(),
                symbols: vec![
                    node(
                        "public struct A",
                        Some("Sources/M/A.swift:1"),
                        vec![node(
                            "public var x: Int",
                            Some("Sources/M/A.swift:2"),
                            vec![],
                        )],
                    ),
                    node("public struct B", Some("Sources/M/Sub/B.swift:1"), vec![]),
                    node("public func orphan()", None, vec![]),
                ],
                extensions: vec![ExtensionGroup {
                    extended_module: "Swift".to_string(),
                    symbols: vec![node(
                        "extension Sequence",
                        None,
                        vec![
                            node("public func a()", Some("Sources/M/A.swift:9"), vec![]),
                            node("public func b()", Some("Sources/M/Sub/B.swift:9"), vec![]),
                        ],
                    )],
                }],
            }],
        }
    }

    #[test]
    fn split_mirrors_source_tree() {
        let files = render_md_split(&split_model());
        let paths: Vec<String> = files.iter().map(|(p, _)| p.display().to_string()).collect();
        assert_eq!(
            paths,
            vec!["index.md", "Sources/M/A.md", "Sources/M/Sub/B.md"]
        );
    }

    #[test]
    fn split_distributes_extension_members_by_file() {
        let files = render_md_split(&split_model());
        let page_a = &files[1].1;
        let page_b = &files[2].1;
        assert!(page_a.contains("public func a()"));
        assert!(!page_a.contains("public func b()"));
        assert!(page_b.contains("public func b()"));
        assert!(page_a.contains("extension Sequence"));
        assert!(page_b.contains("extension Sequence"));
    }

    #[test]
    fn split_index_links_pages_and_holds_sourceless_symbols() {
        let files = render_md_split(&split_model());
        let index = &files[0].1;
        assert!(index.contains("[Sources/M/A.swift](Sources/M/A.md)"));
        assert!(index.contains("[Sources/M/Sub/B.swift](Sources/M/Sub/B.md)"));
        assert!(index.contains("public func orphan()"));
    }

    #[test]
    fn split_rejects_unsafe_source_paths() {
        assert_eq!(
            source_file(&node("d", Some("/abs/File.swift:1"), vec![])),
            None
        );
        assert_eq!(
            source_file(&node("d", Some("../up/File.swift:1"), vec![])),
            None
        );
        assert_eq!(
            source_file(&node("d", Some("Sources/M/F.swift:12"), vec![])),
            Some("Sources/M/F.swift".to_string())
        );
    }
}
