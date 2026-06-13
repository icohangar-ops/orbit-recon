//! Orbit Recon — Automated codebase health analysis using GitLab Orbit Knowledge Graph
//!
//! Reads the DuckDB graph produced by `orbit index` and runs targeted queries
//! to detect dead code, circular dependencies, module coupling, and
//! architectural drift. Outputs structured JSON or Markdown reports.

mod config;
mod findings;
mod queries;
mod report;
mod resilience;

use anyhow::{Context, Result};
use clap::Parser;
use duckdb::Connection;
use std::path::{Path, PathBuf};

/// Orbit Recon: Codebase health analysis via GitLab Orbit Knowledge Graph
#[derive(Parser, Debug)]
#[command(name = "orbit-recon", version, about, long_about = None)]
struct Cli {
    /// Path to the repository (contains .orbit/ directory)
    #[arg(short, long, default_value = ".")]
    repo: PathBuf,

    /// Path to the Orbit DuckDB file (overrides auto-detection)
    #[arg(short = 'd', long)]
    db: Option<PathBuf>,

    /// Output format
    #[arg(short, long, default_value = "markdown", value_parser = ["json", "markdown", "yaml"])]
    format: String,

    /// Output file path (stdout if omitted)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Path to config file (.orbit-recon.yml)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Only run specific checks (comma-separated)
    #[arg(short, long)]
    only: Option<String>,

    /// Minimum severity to report (info, warning, critical)
    #[arg(short = 's', long, default_value = "info")]
    severity: String,

    /// CI mode: exit code 1 if any critical findings
    #[arg(long)]
    ci: bool,
}

fn find_duckdb_path(repo: &Path) -> Result<PathBuf> {
    // Orbit Local stores the graph in .orbit/orbit.duckdb
    let orbit_dir = repo.join(".orbit");
    if !orbit_dir.exists() {
        anyhow::bail!(
            "No .orbit/ directory found in {}. Run `orbit index {}` first.",
            repo.display(),
            repo.display()
        );
    }

    // Check for the standard location
    let db_path = orbit_dir.join("orbit.duckdb");
    if db_path.exists() {
        return Ok(db_path);
    }

    // Try finding any .duckdb file in the orbit directory
    if let Some(entry) = std::fs::read_dir(&orbit_dir)?
        .filter_map(|e| e.ok())
        .find(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "duckdb")
        })
    {
        return Ok(entry.path());
    }

    anyhow::bail!(
        "No DuckDB file found in {}. The Orbit graph may not be indexed.",
        orbit_dir.display()
    )
}

fn main() -> Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("warn"));

    let cli = Cli::parse();

    // Load configuration
    let cfg = if let Some(config_path) = &cli.config {
        config::Config::from_file(config_path)?
    } else {
        let default_config = cli.repo.join(".orbit-recon.yml");
        if default_config.exists() {
            config::Config::from_file(&default_config)?
        } else {
            config::Config::default()
        }
    };

    // Determine which checks to run
    let checks = if let Some(only) = &cli.only {
        only.split(',')
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>()
    } else {
        vec![
            "dead_code".to_string(),
            "circular_dependencies".to_string(),
            "coupling".to_string(),
            "architectural_drift".to_string(),
        ]
    };

    // Find and open the DuckDB database
    let db_path = match &cli.db {
        Some(p) => p.clone(),
        None => find_duckdb_path(&cli.repo)?,
    };

    log::info!("Opening Orbit graph: {}", db_path.display());
    let conn = Connection::open_with_flags(
        &db_path,
        duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?,
    )
    .with_context(|| format!("Failed to open DuckDB: {}", db_path.display()))?;

    let mut all_findings = Vec::new();
    let min_severity = findings::Severity::from_str(&cli.severity);

    // Run each check
    if checks.contains(&"dead_code".to_string()) {
        log::info!("Running dead code detection...");
        let results = queries::dead_code::detect(&conn, &cfg)?;
        all_findings.extend(results);
    }

    if checks.contains(&"circular_dependencies".to_string()) {
        log::info!("Running circular dependency detection...");
        let results = queries::circular_deps::detect(&conn, &cfg)?;
        all_findings.extend(results);
    }

    if checks.contains(&"coupling".to_string()) {
        log::info!("Running module coupling analysis...");
        let results = queries::coupling::analyze(&conn, &cfg)?;
        all_findings.extend(results);
    }

    if checks.contains(&"architectural_drift".to_string()) {
        log::info!("Running architectural drift detection...");
        let results = queries::drift::detect(&conn, &cfg)?;
        all_findings.extend(results);
    }

    // Filter by minimum severity
    all_findings.retain(|f| f.severity >= min_severity);

    // Get graph stats
    let graph_stats = queries::graph_stats(&conn)?;

    // Build the report
    let report = report::Report {
        version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        repository: cli
            .repo
            .canonicalize()
            .unwrap_or(cli.repo.clone())
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        graph_stats,
        findings: all_findings.clone(),
    };

    // Output the report
    let output_str = match cli.format.as_str() {
        "json" => serde_json::to_string_pretty(&report)?,
        "yaml" => serde_yaml::to_string(&report)?,
        "markdown" => report.to_markdown(),
        _ => anyhow::bail!("Unknown format: {}", cli.format),
    };

    match &cli.output {
        Some(path) => {
            std::fs::write(path, &output_str)
                .with_context(|| format!("Failed to write report to {}", path.display()))?;
            log::info!("Report written to {}", path.display());
        }
        None => {
            println!("{}", output_str);
        }
    }

    // CI mode: exit with error code if critical findings exist
    if cli.ci {
        let critical_count = all_findings
            .iter()
            .filter(|f| f.severity == findings::Severity::Critical)
            .count();
        if critical_count > 0 {
            eprintln!(
                "Orbit Recon found {} critical issue(s). Failing CI check.",
                critical_count
            );
            std::process::exit(1);
        }
    }

    Ok(())
}