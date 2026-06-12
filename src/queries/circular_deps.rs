//! Circular dependency detection queries

use crate::config::Config;
use crate::findings::{Category, Finding, RelatedInfo, Severity};
use anyhow::Result;
use duckdb::Connection;
use std::collections::{HashMap, HashSet};

/// Detect bidirectional module dependencies (circular dependencies)
pub fn detect(conn: &Connection, _cfg: &Config) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    // Step 1: Build the module-level dependency graph from the Orbit graph.
    // We extract "module" from the file path by taking the first directory
    // under src/ (or the top-level directory for flat repos).
    //
    // The Orbit graph stores references as (source_definition) -> (target_definition).
    // We aggregate these into module-level edges.

    let sql = r#"
        SELECT
            module_from(source.file) AS from_module,
            module_from(target.file) AS to_module,
            COUNT(*) AS ref_count
        FROM references r
        JOIN definitions source ON r.source_name = source.name AND r.source_file = source.file
        JOIN definitions target ON r.target_name = target.name AND r.target_file = target.file
        WHERE module_from(source.file) != module_from(target.file)
        GROUP BY from_module, to_module
    "#;

    // Since Orbit's DuckDB schema may not have a module_from() function,
    // we do the module extraction in Rust by querying raw references and
    // aggregating ourselves.

    let raw_sql = r#"
        SELECT
            r.source_file,
            r.target_file,
            COUNT(*) AS ref_count
        FROM references r
        GROUP BY r.source_file, r.target_file
    "#;

    // Build module dependency graph
    let mut module_deps: HashMap<String, HashMap<String, i64>> = HashMap::new();

    let mut stmt = conn.prepare(raw_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let src_file: String = row.get(0)?;
            let tgt_file: String = row.get(1)?;
            let count: i64 = row.get(2)?;
            Ok((src_file, tgt_file, count))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (src_file, tgt_file, count) in rows {
        let src_mod = extract_module(&src_file);
        let tgt_mod = extract_module(&tgt_file);

        if src_mod != tgt_mod {
            *module_deps
                .entry(src_mod.clone())
                .or_default()
                .entry(tgt_mod)
                .or_insert(0) += count;
        }
    }

    // Step 2: Find cycles (A -> B and B -> A)
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();

    for (mod_a, deps) in &module_deps {
        for mod_b in deps.keys() {
            if mod_a == mod_b {
                continue;
            }
            let pair = if mod_a < mod_b {
                (mod_a.clone(), mod_b.clone())
            } else {
                (mod_b.clone(), mod_a.clone())
            };

            if seen_pairs.contains(&pair) {
                continue;
            }
            seen_pairs.insert(pair.clone());

            // Check if B -> A also exists
            let forward = module_deps
                .get(&pair.0)
                .and_then(|d| d.get(&pair.1))
                .copied()
                .unwrap_or(0);
            let backward = module_deps
                .get(&pair.1)
                .and_then(|d| d.get(&pair.0))
                .copied()
                .unwrap_or(0);

            if forward > 0 && backward > 0 {
                let total_refs = forward + backward;

                // Severity: more cross-references = more critical
                let severity = if total_refs > 20 {
                    Severity::Critical
                } else {
                    Severity::Warning
                };

                let description = format!(
                    "Circular dependency between `{}` and `{}`: {} cross-references ({} -> {}, {} -> {}).",
                    &pair.0, &pair.1, total_refs, &pair.0, &pair.1, &pair.1, &pair.0
                );

                let remediation = format!(
                    "Extract shared code into a third module that both `{}` and `{}` depend on \
                     (dependency inversion principle). Alternatively, use an event/callback \
                     pattern to break the direct coupling.",
                    &pair.0, &pair.1
                );

                let mut finding = Finding::new(
                    severity,
                    Category::CircularDependency,
                    &format!("{}/", &pair.0),
                    None,
                    &format!("{} <-> {}", &pair.0, &pair.1),
                    &description,
                    &remediation,
                );
                finding.related = Some(RelatedInfo::Cycle {
                    other_module: pair.1.clone(),
                    cycle_count: total_refs,
                });
                findings.push(finding);
            }
        }
    }

    // Step 3: Detect longer cycles (A -> B -> C -> A) using DFS
    let longer_cycles = find_longer_cycles(&module_deps);
    for (cycle_modules, ref_count) in longer_cycles {
        let cycle_str = cycle_modules
            .iter()
            .map(|m| format!("`{}`", m))
            .collect::<Vec<_>>()
            .join(" -> ");

        let description = format!(
            "Dependency cycle involving {} modules: {} -> {} ({} total cross-references). \
             This indicates deeply entangled module boundaries.",
            cycle_modules.len(),
            cycle_str,
            &cycle_modules[0],
            ref_count
        );

        let remediation = format!(
            "This multi-module cycle requires architectural refactoring. Consider introducing \
             an abstraction layer or using dependency injection to break the cycle. Prioritize \
             the weakest coupling point (lowest reference count edge) as the break point."
        );

        let mut finding = Finding::new(
            Severity::Critical,
            Category::CircularDependency,
            &format!("{}/", &cycle_modules[0]),
            None,
            &format!("cycle({})", cycle_modules.len()),
            &description,
            &remediation,
        );
        finding.related = Some(RelatedInfo::Cycle {
            other_module: cycle_str,
            cycle_count: ref_count,
        });
        findings.push(finding);
    }

    Ok(findings)
}

/// Extract a module name from a file path
fn extract_module(file: &str) -> String {
    // Strip leading "./" or "/"
    let cleaned = file
        .strip_prefix("./")
        .or_else(|| file.strip_prefix('/'))
        .unwrap_or(file);

    // Take the first directory component under src/, or the first dir overall
    let parts: Vec<&str> = cleaned.split('/').collect();

    if parts.len() >= 3 && parts[0] == "src" {
        parts[1].to_string()
    } else if parts.len() >= 2 {
        parts[0].to_string()
    } else {
        "root".to_string()
    }
}

/// Find cycles longer than 2 modules using iterative DFS
fn find_longer_cycles(
    deps: &HashMap<String, HashMap<String, i64>>,
) -> Vec<(Vec<String>, i64)> {
    let mut cycles: Vec<(Vec<String>, i64)> = Vec::new();
    let mut visited_global: HashSet<String> = HashSet::new();

    for start in deps.keys() {
        if visited_global.contains(start) {
            continue;
        }

        // DFS to find cycles back to start
        let mut path: Vec<String> = vec![start.clone()];
        let mut path_set: HashSet<String> = HashSet::from([start.clone()]);
        let mut stack: Vec<(String, i64)> = Vec::new();

        // Initialize stack with direct dependencies
        if let Some(neighbors) = deps.get(start) {
            for (neighbor, count) in neighbors {
                if *neighbor != *start {
                    stack.push((neighbor.clone(), *count));
                }
            }
        }

        while let Some((current, count)) = stack.pop() {
            if current == *start && path.len() >= 3 {
                // Found a cycle back to start
                let total_refs: i64 = path
                    .windows(2)
                    .map(|pair| (&pair[0], &pair[1]))
                    .chain(std::iter::once((&path[path.len() - 1], start)))
                    .filter_map(|(from, to)| {
                        deps.get(from)
                            .and_then(|d| d.get(to))
                            .copied()
                    })
                    .sum();
                cycles.push((path.clone(), total_refs));
                continue;
            }

            if path_set.contains(&current) {
                continue;
            }

            path.push(current.clone());
            path_set.insert(current.clone());

            if let Some(neighbors) = deps.get(&current) {
                for (neighbor, _) in neighbors {
                    if !path_set.contains(neighbor) || *neighbor == *start {
                        stack.push((neighbor.clone(), count));
                    }
                }
            }
        }

        visited_global.insert(start.clone());
    }

    // Deduplicate: only keep unique cycles (normalize by rotating to smallest element)
    let mut unique: Vec<(Vec<String>, i64)> = Vec::new();
    let mut seen: HashSet<Vec<String>> = HashSet::new();

    for (mut cycle, count) in cycles {
        if cycle.len() < 3 {
            continue;
        }
        // Rotate so the lexicographically smallest element is first
        if let Some(min_pos) = cycle
            .iter()
            .enumerate()
            .min_by_key(|(_, m)| *m)
            .map(|(i, _)| i)
        {
            cycle.rotate_left(min_pos);
        }
        if seen.insert(cycle.clone()) {
            unique.push((cycle, count));
        }
    }

    unique
}