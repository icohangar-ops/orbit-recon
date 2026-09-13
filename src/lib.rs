//! Orbit Recon library — the shared core behind the `orbit-recon` CLI and the
//! `orbit-recon-mcp` MCP server.
//!
//! The four deterministic health checks (dead code, circular dependencies,
//! module coupling, architectural drift) plus graph statistics are exposed here
//! as plain functions over a read-only DuckDB [`Connection`]. Both the CLI
//! (`src/main.rs`) and the MCP server (`src/bin/mcp_server.rs`) call this same
//! code path — there is no logic duplication between them.

pub mod config;
pub mod findings;
pub mod path_safety;
pub mod queries;
pub mod report;
pub mod resilience;

use anyhow::{Context, Result};
use config::Config;
use duckdb::Connection;
use findings::{Finding, Severity};
use std::path::{Path, PathBuf};

/// The set of health checks Orbit Recon can run. These names are stable and are
/// reused as the `--only` CLI values and the MCP tool identifiers.
pub const ALL_CHECKS: [&str; 4] = [
    "dead_code",
    "circular_dependencies",
    "coupling",
    "architectural_drift",
];

/// Locate the Orbit DuckDB graph for a repository.
///
/// Orbit Local stores the graph in `.orbit/orbit.duckdb`. If that exact file is
/// absent, the first `*.duckdb` file in `.orbit/` is used.
///
/// `repo` is treated as untrusted input (CLI `--repo` / MCP `repo`). `..` is
/// rejected and every join is confined to that repository directory.
pub fn find_duckdb_path(repo: &Path) -> Result<PathBuf> {
    let repo = path_safety::require_no_parent_dir(repo)?;
    let orbit_dir = path_safety::confine_to_base(repo, ".orbit")?;
    if !orbit_dir.exists() {
        anyhow::bail!(
            "No .orbit/ directory found in {}. Run `orbit index {}` first.",
            repo.display(),
            repo.display()
        );
    }

    let db_path = path_safety::confine_to_base(&orbit_dir, "orbit.duckdb")?;
    if db_path.exists() {
        return Ok(db_path);
    }

    if let Some(entry) = std::fs::read_dir(&orbit_dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "duckdb"))
    {
        // Re-confine the directory entry so a crafted name cannot escape.
        let name = entry.file_name();
        return path_safety::confine_to_base(&orbit_dir, &name);
    }

    anyhow::bail!(
        "No DuckDB file found in {}. The Orbit graph may not be indexed.",
        orbit_dir.display()
    )
}

/// Open the Orbit graph read-only.
///
/// `db_path` comes from CLI `--db`, MCP `db`, or [`find_duckdb_path`]. Reject
/// `..` before handing the path to DuckDB.
pub fn open_graph(db_path: &Path) -> Result<Connection> {
    let db_path = path_safety::require_no_parent_dir(db_path)?;
    Connection::open_with_flags(
        db_path,
        duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?,
    )
    .with_context(|| format!("Failed to open DuckDB: {}", db_path.display()))
}

/// Run a single named health check against an open graph.
///
/// Returns an error for an unknown check name so callers (CLI and MCP) surface a
/// clear message rather than silently doing nothing.
pub fn run_check(conn: &Connection, cfg: &Config, check: &str) -> Result<Vec<Finding>> {
    match check {
        "dead_code" => queries::dead_code::detect(conn, cfg),
        "circular_dependencies" => queries::circular_deps::detect(conn, cfg),
        "coupling" => queries::coupling::analyze(conn, cfg),
        "architectural_drift" => queries::drift::detect(conn, cfg),
        other => anyhow::bail!(
            "Unknown check `{}`. Valid checks: {}.",
            other,
            ALL_CHECKS.join(", ")
        ),
    }
}

/// Run a set of checks and return the merged, severity-filtered findings.
///
/// This is the shared driver used by both the CLI and the MCP server so the two
/// entry points stay behaviourally identical.
pub fn run_checks(
    conn: &Connection,
    cfg: &Config,
    checks: &[String],
    min_severity: Severity,
) -> Result<Vec<Finding>> {
    let mut all = Vec::new();
    for check in checks {
        all.extend(run_check(conn, cfg, check)?);
    }
    all.retain(|f| f.severity >= min_severity);
    Ok(all)
}

/// Build a full [`report::Report`] for a repository by opening its graph and
/// running the requested checks. Convenience wrapper for the MCP `health_scan`
/// tool.
pub fn scan_repo(
    repo: &Path,
    db_override: Option<&Path>,
    cfg: &Config,
    checks: &[String],
    min_severity: Severity,
) -> Result<report::Report> {
    let repo = path_safety::require_no_parent_dir(repo)?;
    let db_path = match db_override {
        Some(p) => {
            path_safety::require_no_parent_dir(p)?;
            p.to_path_buf()
        }
        None => find_duckdb_path(repo)?,
    };
    let conn = open_graph(&db_path)?;
    let findings = run_checks(&conn, cfg, checks, min_severity)?;
    let graph_stats = queries::graph_stats(&conn)?;

    Ok(report::Report {
        version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        repository: repo
            .canonicalize()
            .unwrap_or_else(|_| repo.to_path_buf())
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        graph_stats,
        findings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn unique_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orbit-recon-lib-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn find_duckdb_path_rejects_parent_dir_in_repo() {
        let err = find_duckdb_path(Path::new("../outside")).unwrap_err();
        assert!(
            err.to_string().contains("path traversal rejected"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn find_duckdb_path_confines_orbit_join() {
        let dir = unique_temp_dir();
        let orbit = dir.join(".orbit");
        fs::create_dir_all(&orbit).unwrap();
        let db = orbit.join("orbit.duckdb");
        fs::write(&db, []).unwrap();

        let found = find_duckdb_path(&dir).unwrap();
        assert!(found.starts_with(&dir));
        assert!(found.ends_with("orbit.duckdb"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_graph_rejects_parent_dir() {
        let err = open_graph(Path::new("../../etc/passwd")).unwrap_err();
        assert!(
            err.to_string().contains("path traversal rejected"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn scan_repo_rejects_db_override_parent_dir() {
        let dir = unique_temp_dir();
        let err = scan_repo(
            &dir,
            Some(Path::new("../secret.duckdb")),
            &Config::default(),
            &["dead_code".to_string()],
            Severity::Info,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("path traversal rejected"),
            "unexpected error: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
