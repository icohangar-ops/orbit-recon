//! Query module aggregator and graph statistics

use crate::config::Config;
use crate::findings::{Finding, Severity};
use anyhow::Result;
use duckdb::Connection;

pub mod circular_deps;
pub mod coupling;
pub mod dead_code;
pub mod drift;

/// Retrieve basic statistics from the Orbit Knowledge Graph
pub fn graph_stats(conn: &Connection) -> Result<crate::report::GraphStats> {
    // Try to count definitions
    let node_count = try_count_table(conn, "definitions");

    // Try to count references
    let edge_count = try_count_table(conn, "references");

    Ok(crate::report::GraphStats {
        nodes: node_count.unwrap_or(0),
        edges: edge_count.unwrap_or(0),
    })
}

/// Try to count rows in a table, returning None if the table doesn't exist
fn try_count_table(conn: &Connection, table: &str) -> Option<i64> {
    let sql = format!("SELECT COUNT(*) FROM {}", table);
    conn.prepare(&sql)
        .ok()?
        .query_row([], |row| row.get(0))
        .ok()
}

/// Discover the actual DuckDB schema from the Orbit graph
/// and adapt queries to match whatever table/column names Orbit uses.
pub fn discover_schema(conn: &Connection) -> Result<SchemaInfo> {
    let mut tables = Vec::new();

    let sql = "SELECT table_name FROM information_schema.tables WHERE table_schema = 'main'";
    let mut stmt = conn.prepare(sql)?;

    let rows = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            Ok(name)
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for table in rows {
        let columns = get_columns(conn, &table)?;
        tables.push(TableName { name: table, columns });
    }

    Ok(SchemaInfo { tables })
}

struct TableName {
    name: String,
    columns: Vec<String>,
}

pub struct SchemaInfo {
    pub tables: Vec<TableName>,
}

fn get_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT column_name FROM information_schema.columns WHERE table_name = '{}'",
        table
    );
    let mut stmt = conn.prepare(&sql)?;

    let rows = stmt
        .query_map([], |row| {
            let col: String = row.get(0)?;
            Ok(col)
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}