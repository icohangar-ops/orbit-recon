//! Module coupling analysis queries

use crate::config::Config;
use crate::findings::{Category, Finding, RelatedInfo, Severity};
use anyhow::Result;
use duckdb::Connection;
use std::collections::HashMap;

/// Analyze module-level coupling (fan-out metric)
pub fn analyze(conn: &Connection, cfg: &Config) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    // Build module dependency graph from raw references
    let raw_sql = r#"
        SELECT
            r.source_file,
            r.target_file,
            COUNT(*) AS ref_count
        FROM references r
        GROUP BY r.source_file, r.target_file
    "#;

    let mut module_deps: HashMap<String, HashMap<String, i64>> = HashMap::new();

    let mut stmt = conn.prepare(raw_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let src_file: String = row.get(0)?;
            let tgt_file: String = row.get(1)?;
            let _count: i64 = row.get(2)?;
            Ok((src_file, tgt_file))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (src_file, tgt_file) in rows {
        let src_mod = extract_module(&src_file);
        let tgt_mod = extract_module(&tgt_file);

        if src_mod != tgt_mod {
            module_deps
                .entry(src_mod)
                .or_default()
                .entry(tgt_mod)
                .and_modify(|c| *c += 1)
                .or_insert(1);
        }
    }

    // Calculate fan-out for each module
    let mut modules: Vec<(String, usize, Vec<String>)> = module_deps
        .iter()
        .map(|(name, deps)| {
            let sorted_deps: Vec<String> = {
                let mut v: Vec<_> = deps.keys().cloned().collect();
                v.sort();
                v
            };
            (name.clone(), deps.len(), sorted_deps)
        })
        .collect();

    modules.sort_by(|a, b| b.1.cmp(&a.1));

    let warn_threshold = cfg.thresholds.coupling_warning_fan_out;
    let crit_threshold = cfg.thresholds.coupling_critical_fan_out;

    for (module_name, fan_out, dependencies) in modules {
        if fan_out < warn_threshold {
            continue;
        }

        let severity = if fan_out >= crit_threshold {
            Severity::Critical
        } else {
            Severity::Warning
        };

        let deps_display = if dependencies.len() <= 10 {
            dependencies
                .iter()
                .map(|d| format!("`{}`", d))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            let shown = &dependencies[..10];
            let rest = dependencies.len() - 10;
            format!(
                "{} and {} more",
                shown
                    .iter()
                    .map(|d| format!("`{}`", d))
                    .collect::<Vec<_>>()
                    .join(", "),
                rest
            )
        };

        let description = format!(
            "Module `{}` has a fan-out of {}, depending on {} other modules: {}.",
            module_name, fan_out, fan_out, deps_display
        );

        let remediation = if severity == Severity::Critical {
            format!(
                "Module `{}` is heavily coupled to {} other modules. This violates the \
                 stable dependencies principle and makes changes risky. Consider extracting \
                 a facade or interface layer to reduce direct dependencies. Group related \
                 dependencies into sub-modules and route access through a single entry point.",
                module_name, fan_out
            )
        } else {
            format!(
                "Module `{}` has moderate coupling ({} dependencies). Review whether all \
                 dependencies are necessary. Consider using dependency injection or the \
                 mediator pattern to reduce direct coupling.",
                module_name, fan_out
            )
        };

        let mut finding = Finding::new(
            severity,
            Category::HighCoupling,
            &format!("{}/", &module_name),
            None,
            &module_name,
            &description,
            &remediation,
        );
        finding.related = Some(RelatedInfo::Dependencies {
            module_name: module_name.clone(),
            fan_out: fan_out as i64,
            dependencies,
        });
        findings.push(finding);
    }

    Ok(findings)
}

/// Extract a module name from a file path
fn extract_module(file: &str) -> String {
    let cleaned = file
        .strip_prefix("./")
        .or_else(|| file.strip_prefix('/'))
        .unwrap_or(file);

    let parts: Vec<&str> = cleaned.split('/').collect();

    if parts.len() >= 3 && parts[0] == "src" {
        parts[1].to_string()
    } else if parts.len() >= 2 {
        parts[0].to_string()
    } else {
        "root".to_string()
    }
}