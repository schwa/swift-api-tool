//! Package description, symbol graph decoding, and model extraction.

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::model::{ExtensionGroup, ModuleModel, PackageModel, SymbolNode};

#[derive(Parser, Debug)]
pub(crate) struct ExtractionArgs {
    /// Minimum access level (public, package, internal, ...).
    #[arg(long, default_value = "public")]
    min_access_level: String,

    /// Keep the generated symbol graph directory (for debugging).
    #[arg(long)]
    keep_symbols: bool,

    /// List public symbols that lack documentation comments on stderr.
    #[arg(long)]
    report_undocumented: bool,
}

// --- `swift package describe` JSON ---

#[derive(Debug, Deserialize)]
struct PackageDescription {
    name: String,
    targets: Vec<TargetDescription>,
    products: Vec<ProductDescription>,
}

#[derive(Debug, Deserialize)]
struct TargetDescription {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    module_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProductDescription {
    #[allow(dead_code)]
    name: String,
    #[serde(rename = "type")]
    kind: serde_json::Value,
    targets: Vec<String>,
}

impl ProductDescription {
    fn is_library(&self) -> bool {
        self.kind
            .as_object()
            .map(|o| o.contains_key("library"))
            .unwrap_or(false)
    }
}

// --- symbol graph JSON ---

#[derive(Debug, Deserialize)]
struct SymbolGraph {
    #[serde(default)]
    symbols: Vec<Symbol>,
    #[serde(default)]
    relationships: Vec<Relationship>,
}

#[derive(Debug, Deserialize)]
struct Symbol {
    identifier: Identifier,
    kind: Kind,
    #[serde(default, rename = "pathComponents")]
    path_components: Vec<String>,
    #[serde(default, rename = "accessLevel")]
    access_level: String,
    #[serde(default, rename = "declarationFragments")]
    declaration_fragments: Vec<Fragment>,
    #[serde(default, rename = "swiftExtension")]
    swift_extension: Option<SwiftExtension>,
    #[serde(default, rename = "docComment")]
    doc_comment: Option<DocComment>,
    #[serde(default)]
    location: Option<Location>,
}

#[derive(Debug, Deserialize)]
struct DocComment {
    #[serde(default)]
    lines: Vec<DocLine>,
}

#[derive(Debug, Deserialize)]
struct DocLine {
    text: String,
}

#[derive(Debug, Deserialize)]
struct Location {
    uri: String,
    position: Position,
}

#[derive(Debug, Deserialize)]
struct Position {
    line: u32,
}

#[derive(Debug, Deserialize)]
struct Identifier {
    precise: String,
}

#[derive(Debug, Deserialize)]
struct Kind {
    identifier: String,
}

#[derive(Debug, Deserialize)]
struct Fragment {
    spelling: String,
    kind: String,
}

#[derive(Debug, Deserialize)]
struct SwiftExtension {
    #[allow(dead_code)]
    #[serde(rename = "extendedModule")]
    extended_module: Option<String>,
    #[serde(default)]
    constraints: Vec<Constraint>,
}

#[derive(Debug, Deserialize)]
struct Constraint {
    kind: String,
    lhs: String,
    rhs: String,
}

#[derive(Debug, Deserialize)]
struct Relationship {
    source: String,
    target: String,
    kind: String,
}

pub(crate) fn extract_model(package_path: &Path, args: &ExtractionArgs) -> Result<PackageModel> {
    let pkg_path = package_path
        .canonicalize()
        .with_context(|| format!("resolving {}", package_path.display()))?;

    if !pkg_path.join("Package.swift").exists() {
        bail!("no Package.swift at {}", pkg_path.display());
    }

    let description = describe_package(&pkg_path)?;
    let mut library_targets = library_target_names(&description);
    if library_targets.is_empty() {
        bail!("no public library targets found");
    }
    library_targets.sort_unstable();

    let symbols_dir = generate_symbol_graphs(&pkg_path, &library_targets, &args.min_access_level)?;

    let mut modules = Vec::new();
    let source_prefix = format!("file://{}/", pkg_path.display());
    for module in &library_targets {
        modules.push(build_module_model(module, &symbols_dir, &source_prefix)?);
    }

    let model = PackageModel {
        package: description.name.clone(),
        access_level: args.min_access_level.clone(),
        modules,
    };

    if args.report_undocumented {
        report_undocumented(&model);
    }
    if !args.keep_symbols {
        let _ = fs::remove_dir_all(&symbols_dir);
    } else {
        eprintln!("symbol graphs kept at {}", symbols_dir.display());
    }

    Ok(model)
}

fn describe_package(pkg_path: &Path) -> Result<PackageDescription> {
    let out = Command::new("swift")
        .args(["package", "describe", "--type", "json"])
        .current_dir(pkg_path)
        .output()
        .context("running `swift package describe`")?;
    if !out.status.success() {
        bail!(
            "`swift package describe` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    serde_json::from_slice(&out.stdout).context("parsing `swift package describe` JSON")
}

fn library_target_names(desc: &PackageDescription) -> Vec<String> {
    let mut exposed: std::collections::BTreeSet<&str> = Default::default();
    for p in &desc.products {
        if p.is_library() {
            for t in &p.targets {
                exposed.insert(t);
            }
        }
    }
    desc.targets
        .iter()
        .filter(|t| {
            exposed.contains(t.name.as_str())
                && t.kind == "library"
                && t.module_type.as_deref().unwrap_or("SwiftTarget") == "SwiftTarget"
        })
        .map(|t| t.name.clone())
        .collect()
}

fn generate_symbol_graphs(
    pkg_path: &Path,
    library_targets: &[String],
    min_access_level: &str,
) -> Result<PathBuf> {
    let out_dir = pkg_path.join(".build/swift-api-symbols");
    let _ = fs::remove_dir_all(&out_dir);
    fs::create_dir_all(&out_dir)?;

    if min_access_level != "public" {
        bail!("non-public min-access-level not yet implemented");
    }

    let out = Command::new("swift")
        .args(["package", "dump-symbol-graph"])
        .current_dir(pkg_path)
        .output()
        .context("running `swift package dump-symbol-graph`")?;
    // `swift package dump-symbol-graph` also tries to extract test targets,
    // which on fresh CI runners can fail with "Couldn't load module". Treat a
    // non-zero exit as non-fatal if every library target we care about did
    // get a .symbols.json file written.
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        eprintln!(
            "warning: `swift package dump-symbol-graph` exited with a non-zero status. \
             Checking whether library targets were still emitted.\n\n{}",
            stderr
        );
    }

    let build_dir = pkg_path.join(".build");
    let mut found_modules: std::collections::BTreeSet<String> = Default::default();
    visit_files(&build_dir, &mut |entry| {
        let file_name = entry.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let is_symbols = file_name.ends_with(".symbols.json");
        let under_symbolgraph = entry.components().any(|c| c.as_os_str() == "symbolgraph");
        if is_symbols && under_symbolgraph {
            fs::copy(entry, out_dir.join(file_name))?;
            let stem = file_name.trim_end_matches(".symbols.json");
            let module = stem.split('@').next().unwrap_or(stem);
            found_modules.insert(module.to_string());
        }
        Ok(())
    })?;

    let missing: Vec<&String> = library_targets
        .iter()
        .filter(|t| !found_modules.contains(*t))
        .collect();
    if !missing.is_empty() {
        bail!(
            "`swift package dump-symbol-graph` did not emit symbol graphs for: {}\n\n{}",
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            stderr
        );
    }

    Ok(out_dir)
}

/// Depth-first file traversal without collecting the whole tree.
fn visit_files(root: &Path, visit: &mut dyn FnMut(&Path) -> Result<()>) -> Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                visit(&p)?;
            }
        }
    }
    Ok(())
}

// --- Build the tree model from symbol graph files ---

fn build_module_model(
    module: &str,
    symbols_dir: &Path,
    source_prefix: &str,
) -> Result<ModuleModel> {
    let mut own_graph: Option<SymbolGraph> = None;
    let mut ext_graphs: BTreeMap<String, SymbolGraph> = BTreeMap::new();

    for entry in fs::read_dir(symbols_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".symbols.json") {
            continue;
        }
        let stem = name.trim_end_matches(".symbols.json");
        let data = fs::read(entry.path())?;
        let graph: SymbolGraph = serde_json::from_slice(&data)
            .with_context(|| format!("parsing {}", entry.path().display()))?;
        if let Some((lhs, rhs)) = stem.split_once('@') {
            if lhs == module {
                ext_graphs.insert(rhs.to_string(), graph);
            }
        } else if stem == module {
            own_graph = Some(graph);
        }
    }

    let symbols = own_graph
        .map(|g| graph_to_nodes(g, source_prefix))
        .unwrap_or_default();

    let extensions = ext_graphs
        .into_iter()
        .map(|(extended_module, g)| ExtensionGroup {
            extended_module,
            symbols: ext_graph_to_nodes(g, source_prefix),
        })
        .collect();

    Ok(ModuleModel {
        name: module.to_string(),
        symbols,
        extensions,
    })
}

/// Extension graphs only contain the added members; the extended type itself
/// is not emitted as a symbol. Group top-level symbols by their first path
/// component (the extended type) and wrap them in a synthesized
/// `extension <Type>` node.
fn ext_graph_to_nodes(graph: SymbolGraph, source_prefix: &str) -> Vec<SymbolNode> {
    // Filter out synthesized symbols.
    let symbols: Vec<&Symbol> = graph
        .symbols
        .iter()
        .filter(|s| !s.identifier.precise.contains("::SYNTHESIZED::"))
        .collect();

    // Parent map from memberOf relationships.
    let mut parent_of: HashMap<&str, &str> = HashMap::new();
    for r in &graph.relationships {
        if r.kind == "memberOf" && !r.source.contains("::SYNTHESIZED::") {
            parent_of.insert(&r.source, &r.target);
        }
    }

    let by_usr: HashMap<&str, &Symbol> = symbols
        .iter()
        .map(|s| (s.identifier.precise.as_str(), *s))
        .collect();

    let mut children_of: HashMap<&str, Vec<&Symbol>> = HashMap::new();
    let mut roots: Vec<&Symbol> = Vec::new();
    for s in &symbols {
        let usr = s.identifier.precise.as_str();
        match parent_of.get(usr) {
            Some(parent_usr) if by_usr.contains_key(parent_usr) => {
                children_of.entry(*parent_usr).or_default().push(s);
            }
            _ => roots.push(s),
        }
    }

    sort_symbols(&mut roots);
    for v in children_of.values_mut() {
        sort_symbols(v);
    }

    // Group roots by their first path component (the extended type).
    let mut grouped: BTreeMap<String, Vec<&Symbol>> = BTreeMap::new();
    let mut ungrouped: Vec<&Symbol> = Vec::new();
    for r in roots {
        match r.path_components.first() {
            Some(ty) => grouped.entry(ty.clone()).or_default().push(r),
            None => ungrouped.push(r),
        }
    }

    let mut out: Vec<SymbolNode> = grouped
        .into_iter()
        .map(|(ty, members)| SymbolNode {
            decl: format!("extension {}", ty),
            doc: None,
            source: None,
            members: members
                .into_iter()
                .map(|s| symbol_to_node(s, &children_of, source_prefix))
                .collect(),
        })
        .collect();
    for s in ungrouped {
        out.push(symbol_to_node(s, &children_of, source_prefix));
    }
    out
}

fn graph_to_nodes(graph: SymbolGraph, source_prefix: &str) -> Vec<SymbolNode> {
    // Filter out synthesized symbols.
    let symbols: Vec<&Symbol> = graph
        .symbols
        .iter()
        .filter(|s| !s.identifier.precise.contains("::SYNTHESIZED::"))
        .collect();

    // Parent map from memberOf relationships.
    let mut parent_of: HashMap<&str, &str> = HashMap::new();
    for r in &graph.relationships {
        if r.kind == "memberOf" && !r.source.contains("::SYNTHESIZED::") {
            parent_of.insert(&r.source, &r.target);
        }
    }

    let by_usr: HashMap<&str, &Symbol> = symbols
        .iter()
        .map(|s| (s.identifier.precise.as_str(), *s))
        .collect();

    let mut children_of: HashMap<&str, Vec<&Symbol>> = HashMap::new();
    let mut roots: Vec<&Symbol> = Vec::new();
    for s in &symbols {
        let usr = s.identifier.precise.as_str();
        match parent_of.get(usr) {
            Some(parent_usr) if by_usr.contains_key(parent_usr) => {
                children_of.entry(*parent_usr).or_default().push(s);
            }
            _ => roots.push(s),
        }
    }

    sort_symbols(&mut roots);
    for v in children_of.values_mut() {
        sort_symbols(v);
    }

    roots
        .into_iter()
        .map(|s| symbol_to_node(s, &children_of, source_prefix))
        .collect()
}

fn symbol_to_node(
    sym: &Symbol,
    children_of: &HashMap<&str, Vec<&Symbol>>,
    source_prefix: &str,
) -> SymbolNode {
    let members = children_of
        .get(sym.identifier.precise.as_str())
        .map(|kids| {
            kids.iter()
                .map(|k| symbol_to_node(k, children_of, source_prefix))
                .collect()
        })
        .unwrap_or_default();

    SymbolNode {
        decl: render_declaration(sym),
        doc: render_doc(sym),
        source: render_source(sym, source_prefix),
        members,
    }
}

fn render_doc(sym: &Symbol) -> Option<String> {
    let doc = sym.doc_comment.as_ref()?;
    let text = doc
        .lines
        .iter()
        .map(|l| l.text.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Package-relative `path:line` (1-based); absolute path if outside the package.
fn render_source(sym: &Symbol, source_prefix: &str) -> Option<String> {
    let location = sym.location.as_ref()?;
    let path = location
        .uri
        .strip_prefix(source_prefix)
        .or_else(|| location.uri.strip_prefix("file://"))
        .unwrap_or(&location.uri);
    Some(format!("{}:{}", path, location.position.line + 1))
}

fn sort_symbols(v: &mut Vec<&Symbol>) {
    v.sort_by(|a, b| {
        let ak = kind_rank(&a.kind.identifier);
        let bk = kind_rank(&b.kind.identifier);
        ak.cmp(&bk)
            .then_with(|| a.path_components.cmp(&b.path_components))
            .then_with(|| a.identifier.precise.cmp(&b.identifier.precise))
    });
}

fn kind_rank(k: &str) -> u8 {
    match k {
        "swift.protocol" => 0,
        "swift.class" => 1,
        "swift.actor" => 2,
        "swift.struct" => 3,
        "swift.enum" => 4,
        "swift.typealias" => 5,
        "swift.associatedtype" => 6,
        "swift.enum.case" => 7,
        "swift.init" => 8,
        "swift.property" | "swift.var" => 9,
        "swift.type.property" => 10,
        "swift.subscript" => 11,
        "swift.method" => 12,
        "swift.type.method" => 13,
        "swift.func" | "swift.func.op" => 14,
        _ => 100,
    }
}

fn render_declaration(sym: &Symbol) -> String {
    // Join declaration fragments. `public`/`open` keyword is stripped by the
    // extractor; re-insert from accessLevel, *after* any leading attribute
    // fragments (e.g. @MainActor).
    let mut s = String::new();
    let mut inserted = false;
    for f in &sym.declaration_fragments {
        if !inserted && f.kind != "attribute" {
            if !sym.access_level.is_empty() {
                if !s.is_empty() && !s.ends_with(char::is_whitespace) {
                    s.push(' ');
                }
                s.push_str(&sym.access_level);
                s.push(' ');
            }
            inserted = true;
            if f.spelling.chars().all(char::is_whitespace) {
                continue;
            }
        }
        s.push_str(&f.spelling);
    }
    if !inserted && !sym.access_level.is_empty() {
        s.insert_str(0, &format!("{} ", sym.access_level));
    }

    if let Some(ext) = &sym.swift_extension {
        if !ext.constraints.is_empty() && !s.contains(" where ") {
            s.push_str(" where ");
            s.push_str(&render_constraints(&ext.constraints));
        }
    }

    s
}

fn render_constraints(constraints: &[Constraint]) -> String {
    let mut parts: Vec<String> = constraints
        .iter()
        .map(|c| match c.kind.as_str() {
            "conformance" => format!("{}: {}", c.lhs, c.rhs),
            "sameType" => format!("{} == {}", c.lhs, c.rhs),
            other => format!("{} {} {}", c.lhs, other, c.rhs),
        })
        .collect();
    parts.sort();
    parts.join(", ")
}

// --- Undocumented symbol report ---

fn report_undocumented(model: &PackageModel) {
    let mut undocumented: Vec<String> = Vec::new();
    for module in &model.modules {
        collect_undocumented(&module.symbols, &module.name, &mut undocumented);
        for ext in &module.extensions {
            let prefix = format!("{} (extensions to {})", module.name, ext.extended_module);
            collect_undocumented(&ext.symbols, &prefix, &mut undocumented);
        }
    }
    if undocumented.is_empty() {
        eprintln!("all public symbols are documented");
        return;
    }
    eprintln!("undocumented public symbols ({}):", undocumented.len());
    for entry in &undocumented {
        eprintln!("  {}", entry);
    }
}

fn collect_undocumented(nodes: &[SymbolNode], prefix: &str, out: &mut Vec<String>) {
    for node in nodes {
        let first_line = node.decl.lines().next().unwrap_or(&node.decl);
        // Synthesized `extension T` wrappers have no doc or location of their own.
        let synthesized_wrapper = node.source.is_none() && first_line.starts_with("extension ");
        if node.doc.is_none() && !synthesized_wrapper {
            match &node.source {
                Some(source) => out.push(format!("{}: {} ({})", prefix, first_line, source)),
                None => out.push(format!("{}: {}", prefix, first_line)),
            }
        }
        let child_prefix = format!("{}: {}", prefix, symbol_short_name(first_line));
        collect_undocumented(&node.members, &child_prefix, out);
    }
}

/// Trims a declaration line down to a compact `kind Name` label for report paths.
fn symbol_short_name(decl_line: &str) -> String {
    const MODIFIERS: &[&str] = &[
        "public",
        "open",
        "final",
        "static",
        "mutating",
        "nonmutating",
        "override",
        "required",
        "convenience",
        "indirect",
    ];
    let mut tokens = decl_line
        .split_whitespace()
        .skip_while(|t| t.starts_with('@'))
        .skip_while(|t| MODIFIERS.contains(t));
    let kind = tokens.next().unwrap_or("");
    let name = tokens.next().unwrap_or("");
    let name = name.split(['(', '<', ':']).next().unwrap_or(name);
    format!("{} {}", kind, name).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(doc_lines: &[&str], location: Option<(&str, u32)>) -> Symbol {
        Symbol {
            identifier: Identifier {
                precise: "s:test".to_string(),
            },
            kind: Kind {
                identifier: "swift.struct".to_string(),
            },
            path_components: vec!["Widget".to_string()],
            access_level: "public".to_string(),
            declaration_fragments: vec![],
            swift_extension: None,
            doc_comment: if doc_lines.is_empty() {
                None
            } else {
                Some(DocComment {
                    lines: doc_lines
                        .iter()
                        .map(|t| DocLine {
                            text: t.to_string(),
                        })
                        .collect(),
                })
            },
            location: location.map(|(uri, line)| Location {
                uri: uri.to_string(),
                position: Position { line },
            }),
        }
    }

    #[test]
    fn doc_lines_join_and_trim() {
        let s = symbol(&["A widget.", "", "Use it. "], None);
        assert_eq!(render_doc(&s), Some("A widget.\n\nUse it.".to_string()));
    }

    #[test]
    fn empty_doc_is_none() {
        assert_eq!(render_doc(&symbol(&[], None)), None);
        assert_eq!(render_doc(&symbol(&["", "  "], None)), None);
    }

    #[test]
    fn source_is_package_relative_and_one_based() {
        let s = symbol(&[], Some(("file:///pkg/Sources/M/File.swift", 3)));
        assert_eq!(
            render_source(&s, "file:///pkg/"),
            Some("Sources/M/File.swift:4".to_string())
        );
    }

    #[test]
    fn source_outside_package_keeps_absolute_path() {
        let s = symbol(&[], Some(("file:///elsewhere/File.swift", 0)));
        assert_eq!(
            render_source(&s, "file:///pkg/"),
            Some("/elsewhere/File.swift:1".to_string())
        );
    }
    #[test]
    fn undocumented_report_skips_synthesized_extension_wrappers() {
        let nodes = vec![SymbolNode {
            decl: "extension Sequence".to_string(),
            doc: None,
            source: None,
            members: vec![SymbolNode {
                decl: "public func sorted() -> [Element]".to_string(),
                doc: None,
                source: Some("Sources/M/Ext.swift:5".to_string()),
                members: vec![],
            }],
        }];
        let mut out = Vec::new();
        collect_undocumented(&nodes, "M", &mut out);
        assert_eq!(
            out,
            vec![
                "M: extension Sequence: public func sorted() -> [Element] (Sources/M/Ext.swift:5)"
            ]
        );
    }

    #[test]
    fn documented_symbols_not_reported() {
        let nodes = vec![SymbolNode {
            decl: "public struct S".to_string(),
            doc: Some("Documented.".to_string()),
            source: None,
            members: vec![],
        }];
        let mut out = Vec::new();
        collect_undocumented(&nodes, "M", &mut out);
        assert!(out.is_empty());
    }
}
