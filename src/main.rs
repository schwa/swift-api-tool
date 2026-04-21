use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod html;
use html::render_html;

/// Extract public API symbols from a Swift package into a single file.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Path to the Swift package (directory containing Package.swift).
    #[arg(default_value = ".")]
    package_path: PathBuf,

    /// Output file.
    #[arg(short, long, default_value = "public-api.md")]
    output: PathBuf,

    /// Output format. If omitted, inferred from the output file extension.
    #[arg(short, long, value_enum)]
    format: Option<Format>,

    /// Minimum access level (public, package, internal, ...).
    #[arg(long, default_value = "public")]
    min_access_level: String,

    /// Keep the generated symbol graph directory (for debugging).
    #[arg(long)]
    keep_symbols: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Format {
    Md,
    Yaml,
    Html,
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

// --- Intermediate tree model (format-independent) ---

#[derive(Debug, Serialize)]
pub struct PackageModel {
    pub package: String,
    pub access_level: String,
    pub modules: Vec<ModuleModel>,
}

#[derive(Debug, Serialize)]
pub struct ModuleModel {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<SymbolNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<ExtensionGroup>,
}

#[derive(Debug, Serialize)]
pub struct ExtensionGroup {
    pub extended_module: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<SymbolNode>,
}

#[derive(Debug, Serialize)]
pub struct SymbolNode {
    pub decl: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<SymbolNode>,
}

// --- main ---

fn main() -> Result<()> {
    let cli = Cli::parse();
    let pkg_path = cli
        .package_path
        .canonicalize()
        .with_context(|| format!("resolving {}", cli.package_path.display()))?;

    if !pkg_path.join("Package.swift").exists() {
        bail!("no Package.swift at {}", pkg_path.display());
    }

    let format = cli.format.unwrap_or_else(|| infer_format(&cli.output));

    let description = describe_package(&pkg_path)?;
    let library_targets = library_target_names(&description);
    if library_targets.is_empty() {
        bail!("no public library targets found");
    }

    let symbols_dir = generate_symbol_graphs(&pkg_path, &cli.min_access_level)?;

    let mut modules = Vec::new();
    let mut sorted_targets = library_targets.clone();
    sorted_targets.sort();
    for module in &sorted_targets {
        modules.push(build_module_model(module, &symbols_dir)?);
    }

    let model = PackageModel {
        package: description.name.clone(),
        access_level: cli.min_access_level.clone(),
        modules,
    };

    let rendered = match format {
        Format::Md => render_md(&model),
        Format::Yaml => serde_yaml::to_string(&model).context("serializing YAML")?,
        Format::Html => render_html(&model),
    };

    fs::write(&cli.output, rendered)
        .with_context(|| format!("writing {}", cli.output.display()))?;
    eprintln!("wrote {}", cli.output.display());

    if !cli.keep_symbols {
        let _ = fs::remove_dir_all(&symbols_dir);
    } else {
        eprintln!("symbol graphs kept at {}", symbols_dir.display());
    }

    Ok(())
}

fn infer_format(path: &Path) -> Format {
    match path.extension().and_then(|s| s.to_str()) {
        Some("yaml") | Some("yml") => Format::Yaml,
        Some("html") | Some("htm") => Format::Html,
        _ => Format::Md,
    }
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

fn generate_symbol_graphs(pkg_path: &Path, min_access_level: &str) -> Result<PathBuf> {
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
    if !out.status.success() {
        bail!(
            "`swift package dump-symbol-graph` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let build_dir = pkg_path.join(".build");
    let mut found = 0usize;
    for entry in walk(&build_dir) {
        let is_symbols = entry
            .file_name()
            .and_then(|s| s.to_str())
            .map(|n| n.ends_with(".symbols.json"))
            .unwrap_or(false);
        let under_symbolgraph = entry.components().any(|c| c.as_os_str() == "symbolgraph");
        if is_symbols && under_symbolgraph {
            fs::copy(&entry, out_dir.join(entry.file_name().unwrap()))?;
            found += 1;
        }
    }
    if found == 0 {
        bail!("no .symbols.json files were produced");
    }

    Ok(out_dir)
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

// --- Build the tree model from symbol graph files ---

fn build_module_model(module: &str, symbols_dir: &Path) -> Result<ModuleModel> {
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

    let symbols = own_graph.map(graph_to_nodes).unwrap_or_default();

    let extensions = ext_graphs
        .into_iter()
        .map(|(extended_module, g)| ExtensionGroup {
            extended_module,
            symbols: ext_graph_to_nodes(g),
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
fn ext_graph_to_nodes(graph: SymbolGraph) -> Vec<SymbolNode> {
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
            members: members
                .into_iter()
                .map(|s| symbol_to_node(s, &children_of))
                .collect(),
        })
        .collect();
    for s in ungrouped {
        out.push(symbol_to_node(s, &children_of));
    }
    out
}

fn graph_to_nodes(graph: SymbolGraph) -> Vec<SymbolNode> {
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
        .map(|s| symbol_to_node(s, &children_of))
        .collect()
}

fn symbol_to_node(sym: &Symbol, children_of: &HashMap<&str, Vec<&Symbol>>) -> SymbolNode {
    let members = children_of
        .get(sym.identifier.precise.as_str())
        .map(|kids| {
            kids.iter()
                .map(|k| symbol_to_node(k, children_of))
                .collect()
        })
        .unwrap_or_default();

    SymbolNode {
        decl: render_declaration(sym),
        members,
    }
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

// --- Markdown rendering ---

fn render_md(model: &PackageModel) -> String {
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
    let heading = "#".repeat(depth.min(6));
    // Use first line of declaration as the heading subject.
    let first_line = sym.decl.lines().next().unwrap_or("");
    out.push_str(&format!("{heading} `{}`\n\n", first_line));
    out.push_str("```swift\n");
    out.push_str(&sym.decl);
    if !sym.decl.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n\n");
    for child in &sym.members {
        render_md_symbol(child, depth + 1, out);
    }
}
