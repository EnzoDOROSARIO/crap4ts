use clap::{Parser, ValueEnum};
use crap4ts::{analysis, coverage::Coverage, crap_score};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};
use walkdir::{DirEntry, WalkDir};

#[derive(Parser)]
#[command(
    version,
    about = "Report per-function TypeScript complexity, LCOV coverage, and CRAP scores"
)]
struct Args {
    /// Files or directories to analyze (defaults to src)
    #[arg(default_value = "src")]
    paths: Vec<PathBuf>,
    /// LCOV report; otherwise auto-detect coverage/lcov.info
    #[arg(long)]
    lcov: Option<PathBuf>,
    /// Project root for paths and relative LCOV source paths
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
    /// Exit 2 when any score strictly exceeds this value
    #[arg(long, value_parser = threshold)]
    threshold: Option<f64>,
    /// Fail if any analyzed function has unknown coverage
    #[arg(long)]
    require_coverage: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Serialize)]
struct Row {
    file: String,
    #[serde(flatten)]
    function: analysis::Function,
    coverage: Option<f64>,
    crap: Option<f64>,
}

fn threshold(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .ok_or_else(|| "threshold must be a finite nonnegative number".to_owned())
}

fn source_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("ts" | "tsx" | "mts" | "cts")
    ) && ![".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

fn included(entry: &DirEntry) -> bool {
    !entry.file_type().is_dir()
        || !matches!(
            entry.file_name().to_str(),
            Some("node_modules" | ".git" | "dist" | "build" | "coverage" | "target")
        )
}

fn run(args: Args) -> Result<u8, String> {
    let root = fs::canonicalize(&args.root).map_err(|e| format!("{}: {e}", args.root.display()))?;
    let lcov_path = args
        .lcov
        .as_ref()
        .map(|p| root.join(p))
        .unwrap_or_else(|| root.join("coverage/lcov.info"));
    let coverage = match fs::read_to_string(&lcov_path) {
        Ok(text) => Coverage::parse(&text).map_err(|e| format!("{}: {e}", lcov_path.display()))?,
        Err(e) if args.lcov.is_none() && e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("warning: no coverage/lcov.info; coverage and CRAP will be N/A");
            Coverage::default()
        }
        Err(e) => return Err(format!("{}: {e}", lcov_path.display())),
    };
    let mut files = BTreeSet::new();
    for input in &args.paths {
        let path = root.join(input);
        if !path.exists() {
            return Err(format!("source path does not exist: {}", path.display()));
        }
        for entry in WalkDir::new(&path).into_iter().filter_entry(included) {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().is_file() && source_file(entry.path()) {
                files.insert(fs::canonicalize(entry.path()).map_err(|e| e.to_string())?);
            }
        }
    }
    if files.is_empty() {
        return Err("no TypeScript source files found".to_owned());
    }
    let mut rows = Vec::new();
    for file in files {
        let source = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let functions = analysis::analyze(&source, file.extension().is_some_and(|s| s == "tsx"))
            .map_err(|e| format!("{}: {e}", file.display()))?;
        for function in functions {
            let cov = coverage.for_function(&file, &root, function.start_line, function.end_line);
            rows.push(Row {
                file: file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .replace('\\', "/"),
                crap: cov.map(|c| crap_score(function.complexity, c)),
                coverage: cov,
                function,
            });
        }
    }
    rows.sort_by(|a, b| {
        b.crap
            .partial_cmp(&a.crap)
            .unwrap()
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.function.start_line.cmp(&b.function.start_line))
    });
    match args.format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?
        ),
        Format::Text => {
            println!(
                "CRAP Report\n===========\n{:<30} {:<40} {:>4} {:>6} {:>8}",
                "Function", "File:line", "CC", "Cov%", "CRAP"
            );
            for row in &rows {
                println!(
                    "{:<30} {:<40} {:>4} {:>6} {:>8}",
                    row.function.name,
                    format!("{}:{}", row.file, row.function.start_line),
                    row.function.complexity,
                    row.coverage
                        .map(|c| format!("{:.1}", c * 100.0))
                        .unwrap_or_else(|| "N/A".into()),
                    row.crap
                        .map(|c| format!("{c:.1}"))
                        .unwrap_or_else(|| "N/A".into())
                );
            }
        }
    }
    if args.require_coverage && rows.iter().any(|r| r.coverage.is_none()) {
        return Err("coverage is missing for one or more functions".into());
    }
    Ok(
        if args
            .threshold
            .is_some_and(|t| rows.iter().any(|r| r.crap.is_some_and(|c| c > t)))
        {
            2
        } else {
            0
        },
    )
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let code = if error.use_stderr() { 1 } else { 0 };
            let _ = error.print();
            return ExitCode::from(code);
        }
    };
    match run(args) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
