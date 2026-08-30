use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Parser, Debug)]
pub(crate) struct GraphArgs {
    /// Path to the Swift package.
    #[arg(default_value = ".")]
    package_path: PathBuf,

    /// Output file. Supported extensions are .dot, .svg, and .png.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Exclude test targets.
    #[arg(long)]
    skip_test_targets: bool,

    /// Exclude external product dependencies.
    #[arg(long)]
    skip_product_dependencies: bool,
}

#[derive(Debug, Deserialize)]
struct PackageDescription {
    name: String,
    targets: Vec<TargetDescription>,
}

#[derive(Debug, Deserialize)]
struct TargetDescription {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    product_dependencies: Vec<String>,
    #[serde(default)]
    target_dependencies: Vec<String>,
}

pub(crate) fn run(args: &GraphArgs) -> Result<()> {
    let package_path = args
        .package_path
        .canonicalize()
        .with_context(|| format!("resolving {}", args.package_path.display()))?;
    if !package_path.join("Package.swift").exists() {
        bail!("no Package.swift at {}", package_path.display());
    }

    let package = describe_package(&package_path)?;
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("{}.dot", package.name)));
    let dot = render_dot(
        &package,
        args.skip_test_targets,
        args.skip_product_dependencies,
    );
    write_graph(&output, &dot)?;
    eprintln!("wrote {}", output.display());
    Ok(())
}

fn describe_package(package_path: &Path) -> Result<PackageDescription> {
    let output = Command::new("swift")
        .args(["package", "describe", "--type", "json"])
        .current_dir(package_path)
        .output()
        .context("running `swift package describe`")?;
    if !output.status.success() {
        bail!(
            "`swift package describe` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing `swift package describe` JSON")
}

fn render_dot(
    package: &PackageDescription,
    skip_test_targets: bool,
    skip_product_dependencies: bool,
) -> String {
    let mut targets: Vec<&TargetDescription> = package
        .targets
        .iter()
        .filter(|target| !(skip_test_targets && target.kind == "test"))
        .collect();
    targets.sort_by(|left, right| left.name.cmp(&right.name));

    let mut output = String::new();
    writeln!(output, "digraph {} {{", quoted(&package.name)).unwrap();
    for target in targets {
        writeln!(
            output,
            "    {} [color=black, shape=box];",
            quoted(&target.name)
        )
        .unwrap();

        let mut target_dependencies: Vec<&str> = target
            .target_dependencies
            .iter()
            .map(String::as_str)
            .collect();
        target_dependencies.sort_unstable();
        target_dependencies.dedup();
        for dependency in target_dependencies {
            writeln!(
                output,
                "    {} [color=black, shape=box];",
                quoted(&dependency)
            )
            .unwrap();
            writeln!(
                output,
                "    {} -> {};",
                quoted(&target.name),
                quoted(&dependency)
            )
            .unwrap();
        }

        if !skip_product_dependencies {
            let mut product_dependencies: Vec<&str> = target
                .product_dependencies
                .iter()
                .map(String::as_str)
                .collect();
            product_dependencies.sort_unstable();
            product_dependencies.dedup();
            for dependency in product_dependencies {
                writeln!(
                    output,
                    "    {} [color=blue, shape=box];",
                    quoted(&dependency)
                )
                .unwrap();
                writeln!(
                    output,
                    "    {} -> {};",
                    quoted(&target.name),
                    quoted(&dependency)
                )
                .unwrap();
            }
        }
    }
    output.push_str("}\n");
    output
}

fn quoted(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn write_graph(output_path: &Path, dot: &str) -> Result<()> {
    match output_path
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("dot") | None => fs::write(output_path, dot)
            .with_context(|| format!("writing {}", output_path.display())),
        Some(format @ ("svg" | "png")) => render_with_graphviz(output_path, format, dot),
        Some(extension) => {
            bail!("unsupported graph output extension .{extension}; expected .dot, .svg, or .png")
        }
    }
}

fn render_with_graphviz(output_path: &Path, format: &str, dot: &str) -> Result<()> {
    let mut child = Command::new("dot")
        .arg(format!("-T{format}"))
        .arg("-o")
        .arg(output_path)
        .stdin(Stdio::piped())
        .spawn()
        .context("running Graphviz `dot`")?;
    child
        .stdin
        .take()
        .context("opening Graphviz stdin")?
        .write_all(dot.as_bytes())
        .context("writing Graphviz input")?;
    let status = child.wait().context("waiting for Graphviz `dot`")?;
    if !status.success() {
        bail!("Graphviz `dot` failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> PackageDescription {
        PackageDescription {
            name: "Example Package".to_string(),
            targets: vec![
                TargetDescription {
                    name: "ExampleTests".to_string(),
                    kind: "test".to_string(),
                    product_dependencies: vec![],
                    target_dependencies: vec!["Example".to_string()],
                },
                TargetDescription {
                    name: "Example".to_string(),
                    kind: "library".to_string(),
                    product_dependencies: vec!["External".to_string()],
                    target_dependencies: vec!["Support".to_string()],
                },
            ],
        }
    }

    #[test]
    fn renders_target_and_product_dependencies() {
        let dot = render_dot(&package(), false, false);

        assert!(dot.contains("\"Example\" -> \"Support\";"));
        assert!(dot.contains("\"Example\" -> \"External\";"));
        assert!(dot.contains("\"External\" [color=blue, shape=box];"));
    }

    #[test]
    fn filters_tests_and_products() {
        let dot = render_dot(&package(), true, true);

        assert!(!dot.contains("ExampleTests"));
        assert!(!dot.contains("External"));
        assert!(dot.contains("\"Example\" -> \"Support\";"));
    }

    #[test]
    fn quotes_graphviz_identifiers() {
        assert_eq!(quoted("a\\b\"c"), "\"a\\\\b\\\"c\"");
    }
}
