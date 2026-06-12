//! Dead code detection queries

use crate::config::Config;
use crate::findings::{Category, Finding, RelatedInfo, Severity};
use anyhow::Result;
use duckdb::Connection;

/// Detect definitions with no incoming references
pub fn detect(conn: &Connection, cfg: &Config) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    // Query: find definitions that have no incoming REFERENCES edges
    // The Orbit Knowledge Graph stores definitions and cross-file references
    // as a property graph. This query finds nodes with zero in-degree on the
    // REFERENCES edge type.
    let sql = r#"
        SELECT d.name, d.file, d.kind, d.line
        FROM definitions d
        WHERE d.kind IN ('function', 'method', 'class', 'struct', 'enum', 'trait', 'interface')
        AND d.name NOT IN (
            SELECT DISTINCT r.target_name
            FROM references r
        )
        ORDER BY d.file, d.line
    "#;

    let mut stmt = conn.prepare(sql)?;

    let rows = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let file: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let line: Option<i64> = row.get(3)?;
            Ok((name, file, kind, line))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (name, file, kind, line) in rows {
        // Check ignore patterns
        if cfg.should_ignore(&file, "dead_code") {
            continue;
        }

        // Skip index/barrel files and entry points (common false positives)
        let filename = file.rsplit('/').next().unwrap_or(&file);
        if filename == "index.ts"
            || filename == "index.js"
            || filename == "mod.rs"
            || filename == "lib.rs"
            || filename == "main.rs"
        {
            continue;
        }

        // Classify severity
        let is_entry = name == "main"
            || name == "Main"
            || name.starts_with("test_")
            || name.ends_with("_test")
            || name.ends_with("Test")
            || name.ends_with("Spec");

        let severity = if is_entry {
            Severity::Info
        } else if kind == "function" || kind == "method" {
            Severity::Warning
        } else {
            // Public classes/structs/enums with no references are critical
            Severity::Critical
        };

        let description = format!(
            "{} `{}` in {} has no references in the codebase.",
            capitalize_kind(&kind),
            &name,
            &file
        );

        let remediation = if severity == Severity::Critical {
            format!(
                "Public {} `{}` is exported but never used. Consider removing it or \
                 documenting its purpose if it is part of a public API surface.",
                kind.to_lowercase(),
                &name
            )
        } else {
            format!(
                "{} `{}` appears unused. Verify it is not called via reflection, \
                 dynamic dispatch, or plugin registration before removing.",
                capitalize_kind(&kind),
                &name
            )
        };

        let mut finding = Finding::new(
            severity,
            Category::DeadCode,
            &file,
            line,
            &name,
            &description,
            &remediation,
        );
        finding.related = Some(RelatedInfo::DeadCodeKind { kind });
        findings.push(finding);
    }

    Ok(findings)
}

fn capitalize_kind(kind: &str) -> String {
    let mut c = kind.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}