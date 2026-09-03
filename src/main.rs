//! Orbit Recon — Automated codebase health analysis using GitLab Orbit Knowledge Graph
//!
//! Reads the DuckDB graph produced by `orbit index` and runs targeted queries
//! to detect dead code, circular dependencies, module coupling, and
//! architectural drift. Outputs structured JSON or Markdown reports.

use anyhow::{Context, Result};
use clap::Parser;
use orbit_recon::{config, findings, queries, report, ALL_CHECKS};
use std::path::PathBuf;

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
        ALL_CHECKS.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    };

    // Find and open the DuckDB database
    let db_path = match &cli.db {
        Some(p) => p.clone(),
        None => orbit_recon::find_duckdb_path(&cli.repo)?,
    };

    log::info!("Opening Orbit graph: {}", db_path.display());
    let conn = orbit_recon::open_graph(&db_path)?;

    let min_severity = findings::Severity::from_str(&cli.severity);

    // Run the requested checks through the shared driver (same code path the
    // MCP server uses).
    let all_findings = orbit_recon::run_checks(&conn, &cfg, &checks, min_severity)?;

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
