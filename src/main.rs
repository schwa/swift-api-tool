use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod diff;
mod extract;
mod graph;
mod html;
mod model;
mod render_md;

use extract::{extract_model, ExtractionArgs};
use graph::GraphArgs;
use html::render_html;
use render_md::{render_md, render_md_split};

/// Extract the public API surface of a Swift package.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Default subcommand: `api`.
    #[command(flatten)]
    api: ApiArgs,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Export the public API as a YAML snapshot (for diffing in CI).
    Api(ApiArgs),
    /// Render public API documentation as Markdown or HTML.
    Doc(DocArgs),
    /// Compare two YAML API snapshots and print a semantic diff.
    Diff(DiffArgs),
    /// Generate a Swift package dependency graph.
    Graph(GraphArgs),
}

#[derive(Parser, Debug)]
struct ApiArgs {
    /// Path to the Swift package (directory containing Package.swift).
    #[arg(default_value = ".")]
    package_path: PathBuf,

    /// Output file.
    #[arg(short, long, default_value = "public-api.yaml")]
    output: PathBuf,

    #[command(flatten)]
    extraction: ExtractionArgs,
}

#[derive(Parser, Debug)]
struct DocArgs {
    /// Path to the Swift package (directory containing Package.swift).
    #[arg(default_value = ".")]
    package_path: PathBuf,

    /// Output file.
    #[arg(short, long, default_value = "public-api.md")]
    output: PathBuf,

    /// Output format. If omitted, inferred from the output file extension.
    #[arg(short, long, value_enum)]
    format: Option<DocFormat>,

    /// Write a directory of Markdown files mirroring the package's source
    /// tree (one file per Swift source file, plus an index.md). The output
    /// path is treated as the directory root.
    #[arg(long)]
    split: bool,

    #[command(flatten)]
    extraction: ExtractionArgs,
}

#[derive(Parser, Debug)]
struct DiffArgs {
    /// Old (baseline) YAML snapshot.
    old: PathBuf,
    /// New YAML snapshot.
    new: PathBuf,
    /// Output format for the diff report.
    #[arg(long, value_enum, default_value_t = DiffFormat::Text)]
    format: DiffFormat,
    /// Disable ANSI color output (text format only).
    #[arg(long)]
    no_color: bool,
    /// Force ANSI color output even when stdout is not a TTY.
    #[arg(long, conflicts_with = "no_color")]
    color: bool,
    /// Exit 0 if the only differences are additions; non-zero only on
    /// removals or signature changes.
    #[arg(long, alias = "additive-ok", alias = "breaking-only")]
    allow_additive: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum DocFormat {
    Md,
    Html,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum DiffFormat {
    Text,
    Markdown,
}

// --- main ---

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {:#}", e);
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Api(args)) => run_api(args),
        Some(Commands::Diff(args)) => diff::run_diff(args),
        Some(Commands::Graph(args)) => {
            graph::run(args)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Commands::Doc(args)) => run_doc(args),
        None => run_api(&cli.api),
    }
}

fn run_api(args: &ApiArgs) -> Result<ExitCode> {
    let model = extract_model(&args.package_path, &args.extraction)?;
    let rendered = serde_yaml::to_string(&model).context("serializing YAML")?;
    fs::write(&args.output, rendered)
        .with_context(|| format!("writing {}", args.output.display()))?;
    eprintln!("wrote {}", args.output.display());
    Ok(ExitCode::SUCCESS)
}

fn run_doc(args: &DocArgs) -> Result<ExitCode> {
    if args.split {
        if matches!(args.format, Some(DocFormat::Html)) {
            bail!("--split is not supported for HTML output yet");
        }
        let model = extract_model(&args.package_path, &args.extraction)?;
        let files = render_md_split(&model);
        let root = &args.output;
        for (path, content) in &files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&full, content).with_context(|| format!("writing {}", full.display()))?;
        }
        eprintln!("wrote {} files under {}", files.len(), root.display());
        return Ok(ExitCode::SUCCESS);
    }

    let format = args
        .format
        .unwrap_or_else(|| infer_doc_format(&args.output));
    let model = extract_model(&args.package_path, &args.extraction)?;
    let rendered = match format {
        DocFormat::Md => render_md(&model),
        DocFormat::Html => render_html(&model),
    };
    fs::write(&args.output, rendered)
        .with_context(|| format!("writing {}", args.output.display()))?;
    eprintln!("wrote {}", args.output.display());
    Ok(ExitCode::SUCCESS)
}

fn infer_doc_format(path: &Path) -> DocFormat {
    match path.extension().and_then(|s| s.to_str()) {
        Some("html") | Some("htm") => DocFormat::Html,
        _ => DocFormat::Md,
    }
}
