//! Report generation — Markdown and JSON output

use crate::findings::{Category, Finding, Severity};
use serde::Serialize;
use std::fmt;

/// Graph statistics
#[derive(Debug, Serialize)]
pub struct GraphStats {
    pub nodes: i64,
    pub edges: i64,
}

/// The full Orbit Recon report
#[derive(Debug, Serialize)]
pub struct Report {
    pub version: String,
    pub timestamp: String,
    pub repository: String,
    pub graph_stats: GraphStats,
    pub findings: Vec<Finding>,
}

impl Report {
    /// Generate a human-readable Markdown report
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("# Orbit Recon Report\n\n");
        md.push_str(&format!("**Repository**: {}\n", self.repository));
        md.push_str(&format!("**Timestamp**: {}\n", self.timestamp));
        md.push_str(&format!(
            "**Graph Size**: {} nodes, {} edges\n",
            self.graph_stats.nodes, self.graph_stats.edges
        ));
        md.push_str(&format!("**Orbit Recon Version**: {}\n\n", self.version));

        // Summary table
        let dead_code: Vec<_> = self
            .findings
            .iter()
            .filter(|f| matches!(f.category, Category::DeadCode))
            .collect();
        let cycles: Vec<_> = self
            .findings
            .iter()
            .filter(|f| matches!(f.category, Category::CircularDependency))
            .collect();
        let coupling: Vec<_> = self
            .findings
            .iter()
            .filter(|f| matches!(f.category, Category::HighCoupling))
            .collect();
        let drift: Vec<_> = self
            .findings
            .iter()
            .filter(|f| matches!(f.category, Category::ArchitecturalDrift))
            .collect();

        let critical_count = self
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Critical)
            .count();
        let warning_count = self
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count();
        let info_count = self
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .count();

        md.push_str("## Summary\n\n");
        md.push_str("| Category | Count | Worst Severity |\n");
        md.push_str("|---|---|---|\n");
        md.push_str(&format!(
            "| Dead Code | {} | {} |\n",
            dead_code.len(),
            worst_severity(&dead_code)
        ));
        md.push_str(&format!(
            "| Circular Dependencies | {} | {} |\n",
            cycles.len(),
            worst_severity(&cycles)
        ));
        md.push_str(&format!(
            "| High Coupling Modules | {} | {} |\n",
            coupling.len(),
            worst_severity(&coupling)
        ));
        md.push_str(&format!(
            "| Architectural Drift | {} | {} |\n",
            drift.len(),
            worst_severity(&drift)
        ));
        md.push_str("|---|---|---|\n");
        md.push_str(&format!(
            "| **Total** | **{}** | {} critical, {} warning, {} info |\n\n",
            self.findings.len(),
            critical_count,
            warning_count,
            info_count
        ));

        // Detail sections
        if !dead_code.is_empty() {
            md.push_str("## Dead Code\n\n");
            for f in dead_code {
                md.push_str(&format_finding(f));
            }
            md.push('\n');
        }

        if !cycles.is_empty() {
            md.push_str("## Circular Dependencies\n\n");
            for f in cycles {
                md.push_str(&format_finding(f));
            }
            md.push('\n');
        }

        if !coupling.is_empty() {
            md.push_str("## Module Coupling\n\n");
            for f in coupling {
                md.push_str(&format_finding(f));
            }
            md.push('\n');
        }

        if !drift.is_empty() {
            md.push_str("## Architectural Drift\n\n");
            for f in drift {
                md.push_str(&format_finding(f));
            }
            md.push('\n');
        }

        // Recommendations
        if critical_count > 0 || warning_count > 0 {
            md.push_str("## Recommendations\n\n");
            let mut idx = 1;

            if !dead_code.is_empty() {
                md.push_str(&format!(
                    "{}. **Remove dead code**: {} unused definitions found. Start with public \
                     API surfaces (critical) and work inward. Run this check regularly in CI \
                     to prevent accumulation.\n\n",
                    idx,
                    dead_code.len()
                ));
                idx += 1;
            }

            if !cycles.is_empty() {
                md.push_str(&format!(
                    "{}. **Break circular dependencies**: {} cycle(s) detected. Apply the \
                     dependency inversion principle — introduce interfaces or abstract \
                     modules that both sides depend on instead of each other.\n\n",
                    idx,
                    cycles.len()
                ));
                idx += 1;
            }

            if !coupling.is_empty() {
                md.push_str(&format!(
                    "{}. **Reduce module coupling**: {} module(s) with high fan-out. Consider \
                     the facade pattern for high-coupling modules to reduce direct dependency \
                     count and improve change isolation.\n\n",
                    idx,
                    coupling.len()
                ));
                idx += 1;
            }

            if !drift.is_empty() {
                md.push_str(&format!(
                    "{}. **Enforce architecture boundaries**: {} drift violation(s) found. \
                     Add `.orbit-recon.yml` with explicit boundary rules and run this in CI \
                     to prevent new violations from being merged.\n\n",
                    idx,
                    drift.len()
                ));
            }
        } else if self.findings.is_empty() {
            md.push_str("## All Clear\n\n");
            md.push_str(
                "No findings at or above the configured severity threshold. \
                 Your codebase is healthy according to Orbit Recon's analysis.\n",
            );
        }

        md
    }
}

fn worst_severity(findings: &[&Finding]) -> &'static str {
    if findings.iter().any(|f| f.severity == Severity::Critical) {
        "critical"
    } else if findings.iter().any(|f| f.severity == Severity::Warning) {
        "warning"
    } else {
        "info"
    }
}

fn format_finding(f: &Finding) -> String {
    let severity_label = match f.severity {
        Severity::Critical => "**CRITICAL**",
        Severity::Warning => "**WARNING**",
        Severity::Info => "**INFO**",
    };

    let location = match f.location.line {
        Some(line) => format!("{}:{}", f.location.file, line),
        None => f.location.file.clone(),
    };

    format!(
        "- {} [`{}`]({}) — {}\n  - {}\n  - _Suggestion_: {}\n",
        severity_label,
        f.location.name,
        location,
        f.description,
        f.remediation,
    )
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_markdown())
    }
}