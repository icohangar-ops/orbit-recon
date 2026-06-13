//! Architectural drift detection queries

use crate::config::Config;
use crate::findings::{Category, Finding, RelatedInfo, Severity};
use anyhow::Result;
use duckdb::Connection;

/// Detect violations of configured architecture boundaries
pub fn detect(conn: &Connection, cfg: &Config) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    // If no boundary rules are configured, use sensible defaults
    let rules = if cfg.boundaries.is_empty() {
        default_boundaries()
    } else {
        cfg.boundaries.clone()
    };

    // Query all cross-file references. The `LIMIT` is a query size guard
    // (see `resilience::MAX_ROWS`) so a pathologically large reference table
    // in a crafted Orbit graph cannot exhaust memory.
    let sql = format!(
        r#"
        SELECT
            r.source_file,
            r.source_name,
            r.target_file,
            r.target_name
        FROM references r
        WHERE r.source_file != r.target_file
        ORDER BY r.source_file
        LIMIT {}
    "#,
        crate::resilience::MAX_ROWS
    );

    // Guard the (potentially full-table) scan with a wall-clock timeout so a
    // huge or stalled read fails fast instead of hanging the CLI.
    let interrupt = conn.interrupt_handle();
    let rows = crate::resilience::with_read_timeout(interrupt, || {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map([], |row| {
                let src_file: String = row.get(0)?;
                let src_name: String = row.get(1)?;
                let tgt_file: String = row.get(2)?;
                let tgt_name: String = row.get(3)?;
                Ok((src_file, src_name, tgt_file, tgt_name))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;

    for (src_file, src_name, tgt_file, tgt_name) in rows {
        // Check ignore patterns
        if cfg.should_ignore(&src_file, "drift") {
            continue;
        }

        // Find which boundary rule applies to the source file
        let matching_rule = rules
            .iter()
            .find(|rule| {
                glob::Pattern::new(&rule.pattern)
                    .map(|g| g.matches(&src_file))
                    .unwrap_or(false)
            });

        if let Some(rule) = matching_rule {
            // Check if the target file is allowed
            let is_allowed = rule.allowed_imports.iter().any(|pattern| {
                glob::Pattern::new(pattern)
                    .map(|g| g.matches(&tgt_file))
                    .unwrap_or(false)
            });

            if !is_allowed {
                let severity = Severity::Warning;

                let description = format!(
                    "`{}` (in {}, governed by \"{}\" boundary) imports `{}` from `{}`, \
                     which is outside the allowed import scope.",
                    src_name, src_file, rule.name, tgt_name, tgt_file
                );

                let remediation = format!(
                    "Move `{}` to a location within the allowed boundary, or refactor to \
                     use an interface/abstraction defined within the \"{}\" boundary. \
                     Consider applying the dependency inversion principle: define a trait \
                     or protocol inside \"{}\" that `{}` in `{}` can implement.",
                    tgt_name, rule.name, rule.name, tgt_name, tgt_file
                );

                let mut finding = Finding::new(
                    severity,
                    Category::ArchitecturalDrift,
                    &src_file,
                    None,
                    &src_name,
                    &description,
                    &remediation,
                );
                finding.related = Some(RelatedInfo::DriftTarget {
                    target_name: tgt_name,
                    target_file: tgt_file,
                    boundary_name: rule.name.clone(),
                    boundary_rule: rule.pattern.clone(),
                });
                findings.push(finding);
            }
        }
    }

    Ok(findings)
}

/// Default boundary rules for a typical layered architecture
fn default_boundaries() -> Vec<crate::config::BoundaryRule> {
    vec![
        crate::config::BoundaryRule {
            name: "Domain Layer".to_string(),
            pattern: "src/domain/**".to_string(),
            allowed_imports: vec![
                "src/domain/**".to_string(),
                "src/types/**".to_string(),
                "src/models/**".to_string(),
            ],
        },
        crate::config::BoundaryRule {
            name: "Application Layer".to_string(),
            pattern: "src/application/**".to_string(),
            allowed_imports: vec![
                "src/domain/**".to_string(),
                "src/application/**".to_string(),
                "src/types/**".to_string(),
                "src/models/**".to_string(),
            ],
        },
        crate::config::BoundaryRule {
            name: "Presentation Layer".to_string(),
            pattern: "src/presentation/**".to_string(),
            allowed_imports: vec![
                "src/application/**".to_string(),
                "src/domain/**".to_string(),
                "src/types/**".to_string(),
                "src/models/**".to_string(),
            ],
        },
    ]
}