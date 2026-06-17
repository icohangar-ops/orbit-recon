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
    //
    // Orbit's DuckDB schema has no SQL-side `module_from()` function, so we
    // query raw references and do the module extraction in Rust below.

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

/// Find cycles longer than 2 modules (A -> B -> C -> A) using recursive DFS.
///
/// The previous iterative implementation shared a single `path`/`path_set`
/// across the whole stack-based traversal but never popped nodes on backtrack,
/// so cycles reported bogus chains mixing nodes from sibling branches. This
/// recursive version maintains `path`/`path_set` as a proper DFS stack: every
/// node pushed before recursing is popped afterwards, guaranteeing the recorded
/// chain is exactly the simple path from `start` back to `start`.
fn find_longer_cycles(
    deps: &HashMap<String, HashMap<String, i64>>,
) -> Vec<(Vec<String>, i64)> {
    let mut cycles: Vec<(Vec<String>, i64)> = Vec::new();

    // Run a DFS rooted at each node, looking for simple cycles of length >= 3
    // that return to the root.
    for start in deps.keys() {
        let mut path: Vec<String> = vec![start.clone()];
        let mut path_set: HashSet<String> = HashSet::from([start.clone()]);
        dfs_cycles(deps, start, start, &mut path, &mut path_set, &mut cycles);
    }

    // Deduplicate: a length-N cycle is discovered N times (once per rotation
    // as the start node) and also in both traversal directions. Normalize each
    // cycle to a canonical form (rotate so the lexicographically smallest
    // element is first, then pick the lexicographically smaller of the cycle
    // and its reverse) so equivalent cycles collapse to one entry.
    let mut unique: Vec<(Vec<String>, i64)> = Vec::new();
    let mut seen: HashSet<Vec<String>> = HashSet::new();

    for (cycle, count) in cycles {
        if cycle.len() < 3 {
            continue;
        }
        let canonical = canonicalize_cycle(&cycle);
        if seen.insert(canonical) {
            unique.push((cycle, count));
        }
    }

    unique
}

/// Recursive DFS that records every simple path from `start` that returns to
/// `start` with length >= 3. `path`/`path_set` are maintained with strict
/// push-on-descend / pop-on-backtrack discipline.
fn dfs_cycles(
    deps: &HashMap<String, HashMap<String, i64>>,
    start: &str,
    current: &str,
    path: &mut Vec<String>,
    path_set: &mut HashSet<String>,
    cycles: &mut Vec<(Vec<String>, i64)>,
) {
    let neighbors = match deps.get(current) {
        Some(n) => n,
        None => return,
    };

    for neighbor in neighbors.keys() {
        if neighbor == start {
            // A closing edge back to the root forms a cycle. Require length >= 3
            // so that 2-cycles (handled separately as bidirectional pairs) are
            // not double-reported here.
            if path.len() >= 3 {
                let total_refs = cycle_ref_count(deps, path, start);
                cycles.push((path.clone(), total_refs));
            }
            continue;
        }

        if path_set.contains(neighbor) {
            // Revisiting a node already on the path would create a non-simple
            // path; skip it.
            continue;
        }

        path.push(neighbor.clone());
        path_set.insert(neighbor.clone());

        dfs_cycles(deps, start, neighbor, path, path_set, cycles);

        // Backtrack: undo the push so sibling branches see a clean path.
        path.pop();
        path_set.remove(neighbor);
    }
}

/// Sum the edge weights along the cycle path plus the closing edge back to start.
fn cycle_ref_count(
    deps: &HashMap<String, HashMap<String, i64>>,
    path: &[String],
    start: &str,
) -> i64 {
    path.windows(2)
        .map(|pair| (pair[0].as_str(), pair[1].as_str()))
        .chain(std::iter::once((path[path.len() - 1].as_str(), start)))
        .filter_map(|(from, to)| deps.get(from).and_then(|d| d.get(to)).copied())
        .sum()
}

/// Produce a canonical, rotation- and direction-independent key for a cycle so
/// that the same cycle discovered from different start nodes or in the opposite
/// traversal direction deduplicates to a single representation.
fn canonicalize_cycle(cycle: &[String]) -> Vec<String> {
    let rotate_to_min = |seq: &[String]| -> Vec<String> {
        let min_pos = seq
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.cmp(b))
            .map(|(i, _)| i)
            .unwrap_or(0);
        let mut v = seq.to_vec();
        v.rotate_left(min_pos);
        v
    };

    let forward = rotate_to_min(cycle);

    let mut reversed = cycle.to_vec();
    reversed.reverse();
    let backward = rotate_to_min(&reversed);

    if forward <= backward {
        forward
    } else {
        backward
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    /// Build a dependency graph from (from, to, count) triples.
    fn graph(edges: &[(&str, &str, i64)]) -> HashMap<String, HashMap<String, i64>> {
        let mut g: HashMap<String, HashMap<String, i64>> = HashMap::new();
        for (from, to, count) in edges {
            g.entry(s(from)).or_default().insert(s(to), *count);
        }
        g
    }

    #[test]
    fn extract_module_takes_dir_under_src() {
        assert_eq!(extract_module("src/auth/login.rs"), "auth");
        assert_eq!(extract_module("./src/auth/login.rs"), "auth");
        assert_eq!(extract_module("/src/auth/login.rs"), "auth");
    }

    #[test]
    fn extract_module_top_level_dir_for_non_src() {
        assert_eq!(extract_module("lib/db/conn.rs"), "lib");
        assert_eq!(extract_module("pkg/handler.go"), "pkg");
    }

    #[test]
    fn extract_module_flat_file_is_root() {
        assert_eq!(extract_module("main.rs"), "root");
        assert_eq!(extract_module("README.md"), "root");
    }

    #[test]
    fn extract_module_src_with_file_directly_under_src_is_src() {
        // "src/main.rs" has only 2 parts, so it falls through to parts[0] = "src"
        assert_eq!(extract_module("src/main.rs"), "src");
    }

    #[test]
    fn find_longer_cycles_detects_three_node_cycle() {
        // a -> b -> c -> a
        let g = graph(&[("a", "b", 1), ("b", "c", 2), ("c", "a", 3)]);
        let cycles = find_longer_cycles(&g);
        assert_eq!(cycles.len(), 1, "expected exactly one unique 3-cycle");
        let (cycle, count) = &cycles[0];
        assert_eq!(cycle.len(), 3);
        // Total references along the cycle: 1 + 2 + 3 = 6
        assert_eq!(*count, 6);
        // The chain must be a genuine simple cycle containing exactly a, b, c.
        let mut sorted = cycle.clone();
        sorted.sort();
        assert_eq!(sorted, vec![s("a"), s("b"), s("c")]);
    }

    #[test]
    fn find_longer_cycles_ignores_two_node_cycles() {
        // a <-> b is a 2-cycle, handled elsewhere; must NOT appear here.
        let g = graph(&[("a", "b", 5), ("b", "a", 5)]);
        let cycles = find_longer_cycles(&g);
        assert!(cycles.is_empty(), "2-cycles must not be reported as longer cycles");
    }

    #[test]
    fn find_longer_cycles_no_cycle_in_dag() {
        // a -> b -> c, no back edge. Acyclic.
        let g = graph(&[("a", "b", 1), ("b", "c", 1)]);
        let cycles = find_longer_cycles(&g);
        assert!(cycles.is_empty(), "a DAG must yield no cycles");
    }

    #[test]
    fn find_longer_cycles_does_not_mix_sibling_branches() {
        // Regression for the path-tracking bug: 'a' has two branches,
        // a -> b -> a (2-cycle, ignored) and a -> c -> d -> a (3-cycle).
        // The buggy iterative DFS would leak 'b' into the c/d path and
        // report a bogus chain. The correct DFS reports only [a, c, d].
        let g = graph(&[
            ("a", "b", 1),
            ("b", "a", 1),
            ("a", "c", 1),
            ("c", "d", 1),
            ("d", "a", 1),
        ]);
        let cycles = find_longer_cycles(&g);
        assert_eq!(cycles.len(), 1, "exactly one 3-cycle expected");
        let mut sorted = cycles[0].0.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec![s("a"), s("c"), s("d")],
            "cycle must contain exactly a, c, d with no leaked sibling node"
        );
    }

    #[test]
    fn find_longer_cycles_dedupes_rotations() {
        // A single 3-cycle should be reported once even though it is
        // reachable as a starting point from a, b, and c.
        let g = graph(&[("a", "b", 1), ("b", "c", 1), ("c", "a", 1)]);
        let cycles = find_longer_cycles(&g);
        assert_eq!(cycles.len(), 1);
    }

    #[test]
    fn canonicalize_cycle_is_rotation_invariant() {
        let c1 = vec![s("a"), s("b"), s("c")];
        let c2 = vec![s("b"), s("c"), s("a")];
        let c3 = vec![s("c"), s("a"), s("b")];
        assert_eq!(canonicalize_cycle(&c1), canonicalize_cycle(&c2));
        assert_eq!(canonicalize_cycle(&c1), canonicalize_cycle(&c3));
    }

    #[test]
    fn canonicalize_cycle_is_direction_invariant() {
        let forward = vec![s("a"), s("b"), s("c")];
        let backward = vec![s("a"), s("c"), s("b")];
        assert_eq!(canonicalize_cycle(&forward), canonicalize_cycle(&backward));
    }
}