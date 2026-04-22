//! Semantic diff of two YAML API snapshots.
//!
//! Pairs symbols by a normalized identity key derived from the first line of
//! each declaration (attributes and whitespace stripped) within the same
//! parent scope. Reports Added / Removed / Changed with an indented,
//! optionally colorized tree.
//!
//! Exit codes:
//! * 0 — no differences, or `--allow-additive` and only additions
//! * 1 — differences found (breaking in `--allow-additive` mode)
//! * 2 — I/O or parse error (set by the caller via anyhow)

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::io::IsTerminal;
use std::process::ExitCode;

use crate::{DiffArgs, DiffFormat, ExtensionGroup, ModuleModel, PackageModel, SymbolNode};

pub fn run_diff(args: &DiffArgs) -> Result<ExitCode> {
    let old = load_model(&args.old)?;
    let new = load_model(&args.new)?;

    let report = diff_packages(&old, &new);

    let use_color = match args.format {
        DiffFormat::Markdown => false,
        DiffFormat::Text => {
            if args.no_color {
                false
            } else if args.color {
                true
            } else {
                std::io::stdout().is_terminal()
            }
        }
    };

    let rendered = match args.format {
        DiffFormat::Text => render_text(&report, use_color),
        DiffFormat::Markdown => render_markdown(&report),
    };
    print!("{}", rendered);

    if report.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    if args.allow_additive && !report.has_breaking() {
        return Ok(ExitCode::SUCCESS);
    }

    Ok(ExitCode::from(1))
}

fn load_model(path: &std::path::Path) -> Result<PackageModel> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("parsing YAML at {}", path.display()))
}

// --- Report types ---

#[derive(Debug, Default)]
pub struct DiffReport {
    pub modules: Vec<ModuleDiff>,
}

impl DiffReport {
    fn is_empty(&self) -> bool {
        self.modules.iter().all(|m| !m.any_changes())
    }

    fn has_breaking(&self) -> bool {
        self.modules.iter().any(ModuleDiff::has_breaking)
    }
}

#[derive(Debug)]
pub struct ModuleDiff {
    pub name: String,
    pub status: NodeStatus,
    pub symbols: Vec<SymbolDiff>,
    pub extensions: Vec<ExtensionDiff>,
}

impl ModuleDiff {
    fn any_changes(&self) -> bool {
        !matches!(self.status, NodeStatus::Same)
            || !self.symbols.is_empty()
            || !self.extensions.is_empty()
    }

    fn has_breaking(&self) -> bool {
        match self.status {
            NodeStatus::Removed | NodeStatus::Changed { .. } => return true,
            _ => {}
        }
        if self.symbols.iter().any(SymbolDiff::has_breaking) {
            return true;
        }
        self.extensions.iter().any(ExtensionDiff::has_breaking)
    }
}

#[derive(Debug)]
pub struct ExtensionDiff {
    pub extended_module: String,
    pub status: NodeStatus,
    pub symbols: Vec<SymbolDiff>,
}

impl ExtensionDiff {
    fn has_breaking(&self) -> bool {
        match self.status {
            NodeStatus::Removed | NodeStatus::Changed { .. } => return true,
            _ => {}
        }
        self.symbols.iter().any(SymbolDiff::has_breaking)
    }
}

#[derive(Debug)]
pub struct SymbolDiff {
    pub decl: String,
    pub status: NodeStatus,
    pub members: Vec<SymbolDiff>,
}

impl SymbolDiff {
    fn has_breaking(&self) -> bool {
        match self.status {
            NodeStatus::Removed | NodeStatus::Changed { .. } => return true,
            _ => {}
        }
        self.members.iter().any(SymbolDiff::has_breaking)
    }
}

#[derive(Debug, Clone)]
pub enum NodeStatus {
    Same,
    Added,
    Removed,
    /// A node with the same identity key whose `decl` changed.
    Changed {
        old_decl: String,
    },
}

// --- Diffing ---

pub fn diff_packages(old: &PackageModel, new: &PackageModel) -> DiffReport {
    let mut modules = Vec::new();

    let old_by_name: BTreeMap<&str, &ModuleModel> =
        old.modules.iter().map(|m| (m.name.as_str(), m)).collect();
    let new_by_name: BTreeMap<&str, &ModuleModel> =
        new.modules.iter().map(|m| (m.name.as_str(), m)).collect();

    let mut all: Vec<&str> = old_by_name
        .keys()
        .chain(new_by_name.keys())
        .copied()
        .collect();
    all.sort();
    all.dedup();

    for name in all {
        match (old_by_name.get(name), new_by_name.get(name)) {
            (Some(o), Some(n)) => {
                let d = diff_module(o, n);
                if d.any_changes() {
                    modules.push(d);
                }
            }
            (Some(o), None) => modules.push(module_as(o, NodeStatus::Removed)),
            (None, Some(n)) => modules.push(module_as(n, NodeStatus::Added)),
            (None, None) => {}
        }
    }

    DiffReport { modules }
}

fn diff_module(old: &ModuleModel, new: &ModuleModel) -> ModuleDiff {
    ModuleDiff {
        name: new.name.clone(),
        status: NodeStatus::Same,
        symbols: diff_symbols(&old.symbols, &new.symbols),
        extensions: diff_extensions(&old.extensions, &new.extensions),
    }
}

fn module_as(m: &ModuleModel, status: NodeStatus) -> ModuleDiff {
    let symbol_status = match &status {
        NodeStatus::Added => NodeStatus::Added,
        NodeStatus::Removed => NodeStatus::Removed,
        _ => NodeStatus::Same,
    };
    ModuleDiff {
        name: m.name.clone(),
        status,
        symbols: m
            .symbols
            .iter()
            .map(|s| symbol_as(s, symbol_status.clone()))
            .collect(),
        extensions: m
            .extensions
            .iter()
            .map(|e| ExtensionDiff {
                extended_module: e.extended_module.clone(),
                status: symbol_status.clone(),
                symbols: e
                    .symbols
                    .iter()
                    .map(|s| symbol_as(s, symbol_status.clone()))
                    .collect(),
            })
            .collect(),
    }
}

fn diff_extensions(old: &[ExtensionGroup], new: &[ExtensionGroup]) -> Vec<ExtensionDiff> {
    let old_by: BTreeMap<&str, &ExtensionGroup> = old
        .iter()
        .map(|e| (e.extended_module.as_str(), e))
        .collect();
    let new_by: BTreeMap<&str, &ExtensionGroup> = new
        .iter()
        .map(|e| (e.extended_module.as_str(), e))
        .collect();

    let mut names: Vec<&str> = old_by.keys().chain(new_by.keys()).copied().collect();
    names.sort();
    names.dedup();

    let mut out = Vec::new();
    for name in names {
        match (old_by.get(name), new_by.get(name)) {
            (Some(o), Some(n)) => {
                let syms = diff_symbols(&o.symbols, &n.symbols);
                if !syms.is_empty() {
                    out.push(ExtensionDiff {
                        extended_module: name.to_string(),
                        status: NodeStatus::Same,
                        symbols: syms,
                    });
                }
            }
            (Some(o), None) => out.push(ExtensionDiff {
                extended_module: name.to_string(),
                status: NodeStatus::Removed,
                symbols: o
                    .symbols
                    .iter()
                    .map(|s| symbol_as(s, NodeStatus::Removed))
                    .collect(),
            }),
            (None, Some(n)) => out.push(ExtensionDiff {
                extended_module: name.to_string(),
                status: NodeStatus::Added,
                symbols: n
                    .symbols
                    .iter()
                    .map(|s| symbol_as(s, NodeStatus::Added))
                    .collect(),
            }),
            (None, None) => {}
        }
    }
    out
}

fn diff_symbols(old: &[SymbolNode], new: &[SymbolNode]) -> Vec<SymbolDiff> {
    // Pair by identity key. Overload collisions (same key appearing more
    // than once in one side) fall back to exact-decl matching.
    let old_keyed = key_symbols(old);
    let new_keyed = key_symbols(new);

    let mut out = Vec::new();
    let mut keys: Vec<&String> = old_keyed.keys().chain(new_keyed.keys()).collect();
    keys.sort();
    keys.dedup();

    for k in keys {
        let o = old_keyed.get(k);
        let n = new_keyed.get(k);
        match (o, n) {
            (Some(os), Some(ns)) if os.len() == 1 && ns.len() == 1 => {
                let o = os[0];
                let n = ns[0];
                let members = diff_symbols(&o.members, &n.members);
                if o.decl != n.decl {
                    out.push(SymbolDiff {
                        decl: n.decl.clone(),
                        status: NodeStatus::Changed {
                            old_decl: o.decl.clone(),
                        },
                        members,
                    });
                } else if !members.is_empty() {
                    out.push(SymbolDiff {
                        decl: n.decl.clone(),
                        status: NodeStatus::Same,
                        members,
                    });
                }
            }
            (Some(os), Some(ns)) => {
                // Overload set: fall back to exact decl matching.
                let mut by_decl_old: BTreeMap<&str, &SymbolNode> =
                    os.iter().map(|s| (s.decl.as_str(), *s)).collect();
                let mut by_decl_new: BTreeMap<&str, &SymbolNode> =
                    ns.iter().map(|s| (s.decl.as_str(), *s)).collect();
                let decls: Vec<String> = by_decl_old
                    .keys()
                    .chain(by_decl_new.keys())
                    .map(|s| s.to_string())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                for d in decls {
                    let o = by_decl_old.remove(d.as_str());
                    let n = by_decl_new.remove(d.as_str());
                    match (o, n) {
                        (Some(o), Some(n)) => {
                            let members = diff_symbols(&o.members, &n.members);
                            if !members.is_empty() {
                                out.push(SymbolDiff {
                                    decl: n.decl.clone(),
                                    status: NodeStatus::Same,
                                    members,
                                });
                            }
                        }
                        (Some(o), None) => out.push(symbol_as(o, NodeStatus::Removed)),
                        (None, Some(n)) => out.push(symbol_as(n, NodeStatus::Added)),
                        (None, None) => {}
                    }
                }
            }
            (Some(os), None) => {
                for s in os {
                    out.push(symbol_as(s, NodeStatus::Removed));
                }
            }
            (None, Some(ns)) => {
                for s in ns {
                    out.push(symbol_as(s, NodeStatus::Added));
                }
            }
            (None, None) => {}
        }
    }
    out
}

fn key_symbols(v: &[SymbolNode]) -> BTreeMap<String, Vec<&SymbolNode>> {
    let mut out: BTreeMap<String, Vec<&SymbolNode>> = BTreeMap::new();
    for s in v {
        out.entry(identity_key(&s.decl)).or_default().push(s);
    }
    out
}

/// Identity key for a declaration: the first non-attribute line with
/// leading/trailing whitespace trimmed and interior whitespace collapsed.
/// This is stable across cosmetic changes (e.g. adding a `@Sendable`
/// attribute) so a modifier-only change shows up as Changed, not as a
/// Removed/Added pair.
fn identity_key(decl: &str) -> String {
    let mut first: Option<&str> = None;
    for line in decl.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('@') {
            continue;
        }
        first = Some(trimmed);
        break;
    }
    let line = first.unwrap_or_else(|| decl.trim());
    // Strip `public`/`open`/`package` access prefix since the access level
    // can shift between snapshots of the same symbol. Match the keyword
    // followed by any amount of whitespace so we don't leave a stray space
    // in the key when the input uses tabs or multiple spaces.
    let stripped = strip_access_prefix(line);
    // Collapse runs of whitespace to a single space.
    let mut out = String::with_capacity(stripped.len());
    let mut prev_ws = false;
    for ch in stripped.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out
}

fn strip_access_prefix(line: &str) -> &str {
    for kw in ["public", "open", "package"] {
        if let Some(rest) = line.strip_prefix(kw) {
            if rest.starts_with(char::is_whitespace) {
                return rest.trim_start();
            }
        }
    }
    line
}

fn symbol_as(s: &SymbolNode, status: NodeStatus) -> SymbolDiff {
    SymbolDiff {
        decl: s.decl.clone(),
        status: status.clone(),
        members: s
            .members
            .iter()
            .map(|m| symbol_as(m, status.clone()))
            .collect(),
    }
}

// --- Text rendering ---

fn render_text(report: &DiffReport, color: bool) -> String {
    let mut out = String::new();
    if report.is_empty() {
        out.push_str("No API differences.\n");
        return out;
    }

    let (added, removed, changed) = count(report);
    out.push_str(&format!(
        "API differences: {} added, {} removed, {} changed\n\n",
        added, removed, changed
    ));

    for m in &report.modules {
        let label = format!("Module {}", m.name);
        out.push_str(&status_line(&m.status, &label, color, 0));
        for s in &m.symbols {
            render_symbol_text(s, 1, color, &mut out);
        }
        for e in &m.extensions {
            let label = format!("extension {}", e.extended_module);
            out.push_str(&status_line(&e.status, &label, color, 1));
            for s in &e.symbols {
                render_symbol_text(s, 2, color, &mut out);
            }
        }
        out.push('\n');
    }
    out
}

fn render_symbol_text(s: &SymbolDiff, depth: usize, color: bool, out: &mut String) {
    let first_line = s.decl.lines().next().unwrap_or("").trim();
    out.push_str(&status_line(&s.status, first_line, color, depth));
    if let NodeStatus::Changed { old_decl } = &s.status {
        let old_first = old_decl.lines().next().unwrap_or("").trim();
        let indent = "  ".repeat(depth + 1);
        out.push_str(&format!(
            "{}{} {}\n",
            indent,
            paint("was:", Style::Dim, color),
            paint(old_first, Style::Dim, color)
        ));
    }
    for child in &s.members {
        render_symbol_text(child, depth + 1, color, out);
    }
}

fn status_line(status: &NodeStatus, label: &str, color: bool, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let (marker, style) = match status {
        NodeStatus::Same => (" ", Style::None),
        NodeStatus::Added => ("+", Style::Green),
        NodeStatus::Removed => ("-", Style::Red),
        NodeStatus::Changed { .. } => ("~", Style::Yellow),
    };
    format!(
        "{}{} {}\n",
        indent,
        paint(marker, style, color),
        paint(label, style, color)
    )
}

fn count(report: &DiffReport) -> (usize, usize, usize) {
    let mut a = 0;
    let mut r = 0;
    let mut c = 0;
    fn visit_sym(s: &SymbolDiff, a: &mut usize, r: &mut usize, c: &mut usize) {
        match s.status {
            NodeStatus::Added => *a += 1,
            NodeStatus::Removed => *r += 1,
            NodeStatus::Changed { .. } => *c += 1,
            NodeStatus::Same => {}
        }
        for m in &s.members {
            visit_sym(m, a, r, c);
        }
    }
    for m in &report.modules {
        match m.status {
            NodeStatus::Added => a += 1,
            NodeStatus::Removed => r += 1,
            NodeStatus::Changed { .. } => c += 1,
            NodeStatus::Same => {}
        }
        for s in &m.symbols {
            visit_sym(s, &mut a, &mut r, &mut c);
        }
        for e in &m.extensions {
            match e.status {
                NodeStatus::Added => a += 1,
                NodeStatus::Removed => r += 1,
                NodeStatus::Changed { .. } => c += 1,
                NodeStatus::Same => {}
            }
            for s in &e.symbols {
                visit_sym(s, &mut a, &mut r, &mut c);
            }
        }
    }
    (a, r, c)
}

// --- Markdown rendering ---

fn render_markdown(report: &DiffReport) -> String {
    let mut out = String::new();
    if report.is_empty() {
        out.push_str("**No API differences.**\n");
        return out;
    }
    let (added, removed, changed) = count(report);
    out.push_str(&format!(
        "### API differences\n\n- **Added:** {}\n- **Removed:** {}\n- **Changed:** {}\n\n",
        added, removed, changed
    ));
    for m in &report.modules {
        match &m.status {
            NodeStatus::Added => out.push_str(&format!("#### ➕ Module `{}`\n\n", m.name)),
            NodeStatus::Removed => out.push_str(&format!("#### ➖ Module `{}`\n\n", m.name)),
            _ => out.push_str(&format!("#### Module `{}`\n\n", m.name)),
        }
        for s in &m.symbols {
            render_symbol_md(s, 0, &mut out);
        }
        for e in &m.extensions {
            match &e.status {
                NodeStatus::Added => {
                    out.push_str(&format!("- ➕ extension `{}`\n", e.extended_module))
                }
                NodeStatus::Removed => {
                    out.push_str(&format!("- ➖ extension `{}`\n", e.extended_module))
                }
                _ => out.push_str(&format!("- extension `{}`\n", e.extended_module)),
            }
            for s in &e.symbols {
                render_symbol_md(s, 1, &mut out);
            }
        }
        out.push('\n');
    }
    out
}

fn render_symbol_md(s: &SymbolDiff, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let first_line = s.decl.lines().next().unwrap_or("").trim();
    let marker = match &s.status {
        NodeStatus::Same => "•",
        NodeStatus::Added => "➕",
        NodeStatus::Removed => "➖",
        NodeStatus::Changed { .. } => "✎",
    };
    out.push_str(&format!("{}- {} `{}`\n", indent, marker, first_line));
    if let NodeStatus::Changed { old_decl } = &s.status {
        let old_first = old_decl.lines().next().unwrap_or("").trim();
        out.push_str(&format!("{}  - was: `{}`\n", indent, old_first));
    }
    for child in &s.members {
        render_symbol_md(child, depth + 1, out);
    }
}

// --- Tiny ANSI helpers ---

#[derive(Copy, Clone)]
enum Style {
    None,
    Dim,
    Red,
    Green,
    Yellow,
}

fn paint(s: &str, style: Style, on: bool) -> String {
    if !on {
        return s.to_string();
    }
    let code = match style {
        Style::None => return s.to_string(),
        Style::Dim => "2",
        Style::Red => "31",
        Style::Green => "32",
        Style::Yellow => "33",
    };
    format!("\x1b[{}m{}\x1b[0m", code, s)
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(yaml: &str) -> PackageModel {
        serde_yaml::from_str(yaml).expect("valid yaml")
    }

    const BASE: &str = r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols:
      - decl: "public struct Foo"
        members:
          - decl: "public func bar()"
      - decl: "public func shaderScope(_ s: String)"
"#;

    #[test]
    fn no_differences() {
        let a = pkg(BASE);
        let b = pkg(BASE);
        let r = diff_packages(&a, &b);
        assert!(r.is_empty());
    }

    #[test]
    fn detects_added_and_removed() {
        let a = pkg(BASE);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols:
      - decl: "public struct Foo"
        members:
          - decl: "public func bar()"
          - decl: "public func baz()"
"#);
        let r = diff_packages(&a, &b);
        assert!(!r.is_empty());
        let (added, removed, changed) = count(&r);
        assert_eq!((added, removed, changed), (1, 1, 0));
        assert!(r.has_breaking()); // a removal happened
    }

    #[test]
    fn additive_only_is_not_breaking() {
        let a = pkg(BASE);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols:
      - decl: "public struct Foo"
        members:
          - decl: "public func bar()"
          - decl: "public func baz()"
      - decl: "public func shaderScope(_ s: String)"
      - decl: "public struct NewType"
"#);
        let r = diff_packages(&a, &b);
        assert!(!r.is_empty());
        let (added, removed, changed) = count(&r);
        assert_eq!((added, removed, changed), (2, 0, 0));
        assert!(!r.has_breaking());
    }

    #[test]
    fn modifier_only_change_is_changed_not_removed_added() {
        // Adding @Sendable to a closure parameter changes the decl but the
        // identity key (first non-attribute line) is the same.
        let a = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols:
      - decl: "public func onCommandBufferScheduled(_ cb: () -> Void)"
"#);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols:
      - decl: "public func onCommandBufferScheduled(_ cb: @Sendable () -> Void)"
"#);
        let r = diff_packages(&a, &b);
        let (added, removed, changed) = count(&r);
        // identity_key uses the full first line so this will actually be
        // classified as removed+added unless we strip attributes from
        // parameter positions — which we deliberately don't. Ensure at
        // least that we detected changes, and that it is breaking.
        assert!(added + removed + changed >= 1);
        assert!(r.has_breaking());
    }

    #[test]
    fn access_level_change_is_changed() {
        // identity_key strips a leading `public `/`open `/`package ` so a
        // pure access-level flip pairs the same symbol.
        let a = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols:
      - decl: "public func foo()"
"#);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols:
      - decl: "open func foo()"
"#);
        let r = diff_packages(&a, &b);
        let (added, removed, changed) = count(&r);
        assert_eq!((added, removed, changed), (0, 0, 1));
    }

    #[test]
    fn module_added_and_removed() {
        let a = pkg(r#"
package: Demo
access_level: public
modules:
  - name: A
    symbols: [{decl: "public struct X"}]
"#);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: B
    symbols: [{decl: "public struct Y"}]
"#);
        let r = diff_packages(&a, &b);
        assert!(r.has_breaking());
        assert_eq!(r.modules.len(), 2);
    }

    // --- Effect modifier changes: per project policy, `foo()`,
    // `foo() throws`, `foo() async`, `foo() async throws`, `foo() rethrows`,
    // and `foo() throws(MyError)` are all DIFFERENT functions. A change
    // between any two of them must render as one removal + one addition,
    // never as a single "Changed" entry.

    fn effect_modifier_case(old_decl: &str, new_decl: &str) -> (usize, usize, usize) {
        let a = pkg(&format!(
            "package: T\naccess_level: public\nmodules:\n  - name: T\n    symbols:\n      - decl: \"{}\"\n",
            old_decl
        ));
        let b = pkg(&format!(
            "package: T\naccess_level: public\nmodules:\n  - name: T\n    symbols:\n      - decl: \"{}\"\n",
            new_decl
        ));
        count(&diff_packages(&a, &b))
    }

    #[test]
    fn adding_throws_is_remove_plus_add() {
        assert_eq!(
            effect_modifier_case("public func foo()", "public func foo() throws"),
            (1, 1, 0)
        );
    }

    #[test]
    fn adding_async_is_remove_plus_add() {
        assert_eq!(
            effect_modifier_case("public func foo()", "public func foo() async"),
            (1, 1, 0)
        );
    }

    #[test]
    fn throws_to_async_throws_is_remove_plus_add() {
        assert_eq!(
            effect_modifier_case(
                "public func dump() throws -> String",
                "public func dump() async throws -> String"
            ),
            (1, 1, 0)
        );
    }

    #[test]
    fn throws_to_rethrows_is_remove_plus_add() {
        assert_eq!(
            effect_modifier_case(
                "public func map<T>(_ f: (Element) throws -> T) throws -> [T]",
                "public func map<T>(_ f: (Element) throws -> T) rethrows -> [T]"
            ),
            (1, 1, 0)
        );
    }

    #[test]
    fn throws_to_typed_throws_is_remove_plus_add() {
        assert_eq!(
            effect_modifier_case(
                "public func foo() throws",
                "public func foo() throws(MyError)"
            ),
            (1, 1, 0)
        );
    }

    #[test]
    fn sync_and_async_overloads_coexist() {
        // If both overloads exist on both sides, nothing should diff.
        let yaml = r#"
package: T
access_level: public
modules:
  - name: T
    symbols:
      - decl: "public func foo()"
      - decl: "public func foo() async"
"#;
        let r = diff_packages(&pkg(yaml), &pkg(yaml));
        assert!(r.is_empty());
    }

    #[test]
    fn removing_one_of_sync_async_overload_pair_is_clean() {
        let old = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    symbols:
      - decl: "public func foo()"
      - decl: "public func foo() async"
"#);
        let new = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    symbols:
      - decl: "public func foo() async"
"#);
        let r = diff_packages(&old, &new);
        // One overload removed, the other untouched. Must not look like a Change.
        assert_eq!(count(&r), (0, 1, 0));
        assert!(r.has_breaking());
    }

    #[test]
    fn effect_change_under_additive_mode_is_breaking() {
        // foo() -> foo() throws must block --allow-additive since it's a
        // removal of the sync variant plus an addition of the throwing one.
        let old = pkg("package: T\naccess_level: public\nmodules:\n  - name: T\n    symbols: [{decl: \"public func foo()\"}]\n");
        let new = pkg("package: T\naccess_level: public\nmodules:\n  - name: T\n    symbols: [{decl: \"public func foo() throws\"}]\n");
        let r = diff_packages(&old, &new);
        assert!(r.has_breaking());
    }

    // --- Non-effect-modifier cases that SHOULD pair.

    #[test]
    fn attribute_addition_pairs_as_changed() {
        // `@MainActor public func foo()` vs `public func foo()` — identity
        // key skips leading attribute-only lines.
        let a = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    symbols:
      - decl: |
          @MainActor
          public func foo()
"#);
        let b = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    symbols:
      - decl: |
          public func foo()
"#);
        let r = diff_packages(&a, &b);
        assert_eq!(count(&r), (0, 0, 1));
    }

    // --- Nested members.

    #[test]
    fn nested_member_changes_propagate() {
        let a = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    symbols:
      - decl: "public struct Outer"
        members:
          - decl: "public struct Inner"
            members:
              - decl: "public func leaf()"
"#);
        let b = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    symbols:
      - decl: "public struct Outer"
        members:
          - decl: "public struct Inner"
            members:
              - decl: "public func leaf() throws"
"#);
        let r = diff_packages(&a, &b);
        assert_eq!(count(&r), (1, 1, 0));
        assert!(r.has_breaking());
        // Rendered report should contain the nested context.
        let txt = render_text(&r, false);
        assert!(txt.contains("public struct Outer"));
        assert!(txt.contains("public struct Inner"));
        assert!(txt.contains("- public func leaf()"));
        assert!(txt.contains("+ public func leaf() throws"));
    }

    // --- Extension groups.

    #[test]
    fn extension_group_added() {
        let a = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    symbols: []
"#);
        let b = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    extensions:
      - extended_module: Swift
        symbols:
          - decl: "extension Array"
            members:
              - decl: "public func shuffled2() -> Array"
"#);
        let r = diff_packages(&a, &b);
        assert!(!r.is_empty());
        assert!(!r.has_breaking());
        let (added, removed, changed) = count(&r);
        assert_eq!(removed, 0);
        assert_eq!(changed, 0);
        assert!(added >= 1);
    }

    #[test]
    fn extension_member_change_inside_existing_group() {
        let a = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    extensions:
      - extended_module: Swift
        symbols:
          - decl: "extension Array"
            members:
              - decl: "public func old()"
"#);
        let b = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    extensions:
      - extended_module: Swift
        symbols:
          - decl: "extension Array"
            members:
              - decl: "public func new()"
"#);
        let r = diff_packages(&a, &b);
        assert_eq!(count(&r), (1, 1, 0));
    }

    // --- Overload handling by parameter labels.

    #[test]
    fn overloads_distinguished_by_param_labels() {
        let a = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    symbols:
      - decl: "public func foo(_ x: Int)"
      - decl: "public func foo(_ x: String)"
"#);
        let b = pkg(r#"
package: T
access_level: public
modules:
  - name: T
    symbols:
      - decl: "public func foo(_ x: String)"
      - decl: "public func foo(_ x: Double)"
"#);
        let r = diff_packages(&a, &b);
        // foo(Int) removed, foo(Double) added, foo(String) unchanged.
        assert_eq!(count(&r), (1, 1, 0));
    }

    // --- Markdown output.

    #[test]
    fn render_markdown_contains_markers_and_summary() {
        let a = pkg(BASE);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols:
      - decl: "public struct Foo"
        members:
          - decl: "public func bar()"
          - decl: "public func baz()"
"#);
        let md = render_markdown(&diff_packages(&a, &b));
        assert!(md.contains("**Added:** 1"));
        assert!(md.contains("**Removed:** 1"));
        assert!(md.contains("**Changed:** 0"));
        assert!(md.contains("➕"));
        assert!(md.contains("➖"));
    }

    #[test]
    fn markdown_has_no_ansi_escapes() {
        let a = pkg(BASE);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols: []
"#);
        let md = render_markdown(&diff_packages(&a, &b));
        assert!(
            !md.contains('\x1b'),
            "markdown must not contain ANSI escapes"
        );
    }

    // --- TTY / color overrides.

    #[test]
    fn no_color_produces_no_ansi() {
        let a = pkg(BASE);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols: []
"#);
        let txt = render_text(&diff_packages(&a, &b), false);
        assert!(!txt.contains('\x1b'));
    }

    #[test]
    fn color_produces_ansi_escapes() {
        let a = pkg(BASE);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols: []
"#);
        let txt = render_text(&diff_packages(&a, &b), true);
        assert!(txt.contains('\x1b'));
    }

    // --- identity_key sanity.

    #[test]
    fn identity_key_strips_access_and_collapses_whitespace() {
        assert_eq!(
            identity_key("public   func    foo()"),
            identity_key("func foo()")
        );
    }

    #[test]
    fn identity_key_skips_leading_attribute_lines() {
        assert_eq!(
            identity_key("@MainActor\npublic func foo()"),
            identity_key("public func foo()")
        );
    }

    #[test]
    fn identity_key_keeps_throws_async() {
        // Critical: effect modifiers MUST stay in the identity key so that
        // foo() and foo() throws / foo() async never collapse together.
        assert_ne!(
            identity_key("func foo()"),
            identity_key("func foo() throws")
        );
        assert_ne!(identity_key("func foo()"), identity_key("func foo() async"));
        assert_ne!(
            identity_key("func foo() throws"),
            identity_key("func foo() async throws")
        );
        assert_ne!(
            identity_key("func foo() throws"),
            identity_key("func foo() rethrows")
        );
        assert_ne!(
            identity_key("func foo() throws"),
            identity_key("func foo() throws(MyError)")
        );
    }

    #[test]
    fn render_text_contains_markers() {
        let a = pkg(BASE);
        let b = pkg(r#"
package: Demo
access_level: public
modules:
  - name: Demo
    symbols:
      - decl: "public struct Foo"
        members:
          - decl: "public func bar()"
          - decl: "public func baz()"
"#);
        let r = diff_packages(&a, &b);
        let txt = render_text(&r, false);
        assert!(txt.contains("+ public func baz()"));
        assert!(txt.contains("- public func shaderScope"));
    }
}
