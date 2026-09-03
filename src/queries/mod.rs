//! Query module aggregator and graph statistics

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

/// Validate that an identifier (table or column name) is safe to interpolate
/// into a SQL string. DuckDB does not support parameterizing identifiers, so we
/// allowlist-validate against `[A-Za-z0-9_]+` to prevent SQL injection from an
/// adversarially-named table in a maliciously crafted orbit.duckdb file.
fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Try to count rows in a table, returning None if the table doesn't exist
fn try_count_table(conn: &Connection, table: &str) -> Option<i64> {
    if !is_safe_identifier(table) {
        return None;
    }
    // Quote the identifier so tables whose names collide with SQL reserved
    // keywords (e.g. `references`) parse correctly. `is_safe_identifier` has
    // already constrained `table` to `[A-Za-z0-9_]+`, so embedding it inside
    // double quotes cannot break out of the quoted identifier.
    let sql = format!(r#"SELECT COUNT(*) FROM "{}""#, table);
    conn.prepare(&sql)
        .ok()?
        .query_row([], |row| row.get(0))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::is_safe_identifier;

    #[test]
    fn accepts_plain_identifiers() {
        assert!(is_safe_identifier("definitions"));
        assert!(is_safe_identifier("references"));
        assert!(is_safe_identifier("Table_1"));
        assert!(is_safe_identifier("_internal"));
    }

    #[test]
    fn rejects_empty() {
        assert!(!is_safe_identifier(""));
    }

    #[test]
    fn rejects_sql_injection_payloads() {
        // These are the kinds of adversarial table names that could appear in a
        // maliciously crafted orbit.duckdb file.
        assert!(!is_safe_identifier("definitions; DROP TABLE references"));
        assert!(!is_safe_identifier("foo WHERE 1=1"));
        assert!(!is_safe_identifier("t'--"));
        assert!(!is_safe_identifier("a b"));
        assert!(!is_safe_identifier("foo)"));
        assert!(!is_safe_identifier("schema.table"));
    }
}
