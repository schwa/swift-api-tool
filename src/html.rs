use crate::{PackageModel, SymbolNode};
use std::fmt::Write;

pub fn render_html(model: &PackageModel) -> String {
    let mut body = String::new();
    let mut nav = String::new();

    writeln!(nav, "<ul class=\"nav-root\">").ok();

    for (mi, m) in model.modules.iter().enumerate() {
        let module_id = format!("mod-{}", mi);
        writeln!(
            nav,
            "<li><a href=\"#{id}\">{name}</a>",
            id = html_escape(&module_id),
            name = html_escape(&m.name)
        )
        .ok();

        writeln!(
            body,
            "<section class=\"module\" id=\"{id}\"><h2>{name}</h2>",
            id = html_escape(&module_id),
            name = html_escape(&m.name)
        )
        .ok();

        if !m.symbols.is_empty() {
            writeln!(nav, "<ul>").ok();
            render_nav_nodes(&m.symbols, &module_id, &mut nav);
            writeln!(nav, "</ul>").ok();

            writeln!(body, "<div class=\"symbols\">").ok();
            for (i, s) in m.symbols.iter().enumerate() {
                render_node(s, &format!("{}-s{}", module_id, i), &mut body);
            }
            writeln!(body, "</div>").ok();
        }

        for (ei, ext) in m.extensions.iter().enumerate() {
            let ext_id = format!("{}-e{}", module_id, ei);
            writeln!(
                nav,
                "<li><a href=\"#{id}\">ext: {name}</a>",
                id = html_escape(&ext_id),
                name = html_escape(&ext.extended_module)
            )
            .ok();
            writeln!(nav, "<ul>").ok();
            render_nav_nodes(&ext.symbols, &ext_id, &mut nav);
            writeln!(nav, "</ul></li>").ok();

            writeln!(
                body,
                "<section class=\"ext\" id=\"{id}\"><h3>Extensions to <code>{name}</code></h3>",
                id = html_escape(&ext_id),
                name = html_escape(&ext.extended_module)
            )
            .ok();
            writeln!(body, "<div class=\"symbols\">").ok();
            for (i, s) in ext.symbols.iter().enumerate() {
                render_node(s, &format!("{}-s{}", ext_id, i), &mut body);
            }
            writeln!(body, "</div></section>").ok();
        }

        writeln!(body, "</section>").ok();
        writeln!(nav, "</li>").ok();
    }
    writeln!(nav, "</ul>").ok();

    let title = format!("{} – Public API", model.package);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title}</title>
<style>{css}</style>
</head>
<body>
<header>
  <h1>{pkg}</h1>
  <p class="meta">Access level: <code>{al}</code></p>
  <input type="search" id="filter" placeholder="Filter symbols…" autocomplete="off" autofocus>
</header>
<div class="layout">
  <nav id="nav">{nav}</nav>
  <main>{body}</main>
</div>
<script>{js}</script>
</body>
</html>
"#,
        title = html_escape(&title),
        pkg = html_escape(&model.package),
        al = html_escape(&model.access_level),
        css = CSS,
        js = JS,
        nav = nav,
        body = body,
    )
}

fn render_node(node: &SymbolNode, id: &str, out: &mut String) {
    let name = symbol_name(&node.decl);
    if node.members.is_empty() {
        writeln!(
            out,
            "<div class=\"sym leaf\" id=\"{id}\" data-name=\"{name_attr}\"><pre><code>{decl}</code></pre></div>",
            id = html_escape(id),
            name_attr = html_escape(&name.to_lowercase()),
            decl = html_escape(node.decl.trim_end())
        )
        .ok();
    } else {
        writeln!(
            out,
            "<details class=\"sym\" id=\"{id}\" data-name=\"{name_attr}\" open><summary><pre><code>{decl}</code></pre></summary><div class=\"children\">",
            id = html_escape(id),
            name_attr = html_escape(&name.to_lowercase()),
            decl = html_escape(node.decl.trim_end())
        )
        .ok();
        for (i, child) in node.members.iter().enumerate() {
            render_node(child, &format!("{}-m{}", id, i), out);
        }
        writeln!(out, "</div></details>").ok();
    }
}

fn render_nav_nodes(nodes: &[SymbolNode], parent_id: &str, out: &mut String) {
    for (i, n) in nodes.iter().enumerate() {
        let id = format!("{}-s{}", parent_id, i);
        let name = symbol_name(&n.decl);
        if n.members.is_empty() {
            writeln!(
                out,
                "<li><a href=\"#{id}\">{name}</a></li>",
                id = html_escape(&id),
                name = html_escape(&name)
            )
            .ok();
        } else {
            writeln!(
                out,
                "<li><a href=\"#{id}\">{name}</a><ul>",
                id = html_escape(&id),
                name = html_escape(&name)
            )
            .ok();
            render_nav_nodes_inner(&n.members, &id, out);
            writeln!(out, "</ul></li>").ok();
        }
    }
}

fn render_nav_nodes_inner(nodes: &[SymbolNode], parent_id: &str, out: &mut String) {
    for (i, n) in nodes.iter().enumerate() {
        let id = format!("{}-m{}", parent_id, i);
        let name = symbol_name(&n.decl);
        if n.members.is_empty() {
            writeln!(
                out,
                "<li><a href=\"#{id}\">{name}</a></li>",
                id = html_escape(&id),
                name = html_escape(&name)
            )
            .ok();
        } else {
            writeln!(
                out,
                "<li><a href=\"#{id}\">{name}</a><ul>",
                id = html_escape(&id),
                name = html_escape(&name)
            )
            .ok();
            render_nav_nodes_inner(&n.members, &id, out);
            writeln!(out, "</ul></li>").ok();
        }
    }
}

/// Best-effort extraction of a short symbol name from a declaration, used
/// for nav labels and search indexing.
fn symbol_name(decl: &str) -> String {
    // Strip leading attributes (`@Foo`) and whitespace/newlines.
    let decl = decl.lines().next().unwrap_or(decl).trim();
    let mut tokens = decl.split_whitespace().peekable();
    while let Some(&t) = tokens.peek() {
        if t.starts_with('@') {
            tokens.next();
        } else {
            break;
        }
    }
    const SKIP: &[&str] = &[
        "public",
        "open",
        "package",
        "internal",
        "fileprivate",
        "private",
        "static",
        "class",
        "final",
        "override",
        "mutating",
        "nonmutating",
        "lazy",
        "weak",
        "unowned",
        "dynamic",
        "convenience",
        "required",
        "indirect",
        "async",
        "throws",
        "rethrows",
        "prefix",
        "postfix",
        "infix",
    ];
    while let Some(&t) = tokens.peek() {
        if SKIP.contains(&t) {
            tokens.next();
        } else {
            break;
        }
    }
    // Next token is typically the kind keyword (func/var/struct/...).
    let kind = tokens.next().unwrap_or("");
    // Then the identifier (up to `(`, `:`, `<`, `=`, whitespace).
    let rest: String = tokens.collect::<Vec<_>>().join(" ");
    let name_end = rest
        .find(|c: char| {
            c == '(' || c == ':' || c == '<' || c == '=' || c == '{' || c.is_whitespace()
        })
        .unwrap_or(rest.len());
    let ident = rest[..name_end].trim_end_matches(':');
    let signature = if let Some(open) = rest.find('(') {
        if let Some(close) = rest[open..].find(')') {
            &rest[open..=open + close]
        } else {
            ""
        }
    } else {
        ""
    };
    let label_sig = abbreviate_signature(signature);
    if ident.is_empty() {
        kind.to_string()
    } else if label_sig.is_empty() {
        format!("{} {}", kind, ident).trim().to_string()
    } else {
        format!("{} {}{}", kind, ident, label_sig)
            .trim()
            .to_string()
    }
}

/// Turn `(x: Int, y: Int)` into `(x:y:)` for compact nav labels.
fn abbreviate_signature(sig: &str) -> String {
    if sig.is_empty() || !sig.starts_with('(') {
        return String::new();
    }
    let inner = &sig[1..sig.len().saturating_sub(1)];
    if inner.trim().is_empty() {
        return "()".to_string();
    }
    let parts: Vec<String> = inner
        .split(',')
        .map(|p| {
            let p = p.trim();
            // Label is the first identifier; if there's an external+internal
            // pair like `to output`, take the external.
            let first = p
                .split(|c: char| c == ':' || c.is_whitespace())
                .next()
                .unwrap_or("_");
            format!("{}:", first)
        })
        .collect();
    format!("({})", parts.join(""))
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const CSS: &str = r#"
* { box-sizing: border-box; }
body {
  margin: 0;
  font: 14px/1.4 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  color: #222;
  background: #fafafa;
}
header {
  position: sticky; top: 0; z-index: 10;
  background: #fff;
  border-bottom: 1px solid #ddd;
  padding: 10px 20px;
  display: flex; align-items: center; gap: 20px;
}
header h1 { font-size: 18px; margin: 0; }
header .meta { margin: 0; color: #666; font-size: 12px; }
header input[type=search] {
  margin-left: auto;
  padding: 6px 10px;
  border: 1px solid #ccc;
  border-radius: 4px;
  font: inherit;
  width: 280px;
}
.layout { display: flex; height: calc(100vh - 52px); }
nav#nav {
  width: 320px;
  overflow-y: auto;
  border-right: 1px solid #ddd;
  padding: 12px;
  background: #fff;
  font-size: 13px;
}
nav ul { list-style: none; padding-left: 14px; margin: 2px 0; }
nav > ul.nav-root { padding-left: 0; }
nav a {
  color: #333;
  text-decoration: none;
  display: block;
  padding: 1px 4px;
  border-radius: 3px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
nav a:hover { background: #eef; }
nav li.hidden { display: none; }
main { flex: 1; overflow-y: auto; padding: 20px 30px; }
section.module > h2 {
  margin-top: 0;
  border-bottom: 2px solid #333;
  padding-bottom: 4px;
}
section.ext > h3 {
  margin-top: 24px;
  color: #555;
  border-bottom: 1px dashed #aaa;
  padding-bottom: 2px;
}
.symbols { margin-left: 0; }
.sym {
  margin: 6px 0;
  padding-left: 8px;
  border-left: 2px solid #ddd;
}
.sym.hidden { display: none; }
.sym pre {
  margin: 0;
  background: #f0f0f0;
  padding: 4px 8px;
  border-radius: 3px;
  overflow-x: auto;
  font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace;
}
details.sym > summary {
  cursor: pointer;
  list-style: none;
}
details.sym > summary::-webkit-details-marker { display: none; }
details.sym > summary::before {
  content: "▸ ";
  color: #888;
  font-size: 10px;
}
details.sym[open] > summary::before { content: "▾ "; }
.children { margin-left: 12px; margin-top: 4px; }
.sym:target > pre, .sym:target > summary > pre {
  background: #fff8c5;
}
"#;

const JS: &str = r#"
(function() {
  const filter = document.getElementById('filter');
  if (!filter) return;

  function apply(q) {
    q = q.trim().toLowerCase();
    // Show/hide symbols in main.
    document.querySelectorAll('.sym').forEach(el => {
      const name = el.dataset.name || '';
      const match = !q || name.includes(q);
      el.classList.toggle('hidden', !match);
      if (match && q) {
        // Ensure ancestor <details> are open so match is visible.
        let p = el.parentElement;
        while (p) {
          if (p.tagName === 'DETAILS') p.open = true;
          p = p.parentElement;
        }
      }
    });
    // Show/hide nav entries whose link points to a visible symbol.
    document.querySelectorAll('nav li').forEach(li => {
      const a = li.querySelector(':scope > a');
      if (!a) return;
      const href = a.getAttribute('href');
      if (!href || !href.startsWith('#')) return;
      const target = document.getElementById(href.slice(1));
      // Keep module/extension-level entries always visible.
      if (target && (target.classList.contains('module') || target.classList.contains('ext'))) {
        li.classList.remove('hidden');
        return;
      }
      const visible = target && !target.classList.contains('hidden');
      li.classList.toggle('hidden', !visible);
    });
  }

  filter.addEventListener('input', e => apply(e.target.value));
})();
"#;
