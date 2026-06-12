//! Finding types and severity classification

use serde::Serialize;

/// Severity levels for findings
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "critical" => Severity::Critical,
            "warning" => Severity::Warning,
            _ => Severity::Info,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
}

/// Category of a finding
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    DeadCode,
    CircularDependency,
    HighCoupling,
    ArchitecturalDrift,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::DeadCode => "dead_code",
            Category::CircularDependency => "circular_dependency",
            Category::HighCoupling => "high_coupling",
            Category::ArchitecturalDrift => "architectural_drift",
        }
    }
}

/// Location of a finding in the codebase
#[derive(Debug, Clone, Serialize)]
pub struct Location {
    pub file: String,
    pub line: Option<i64>,
    pub name: String,
}

/// A single finding from an analysis check
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub category: Category,
    pub location: Location,
    pub description: String,
    pub remediation: String,
    /// Additional context (e.g., the other module in a cycle)
    pub related: Option<RelatedInfo>,
}

/// Extra context attached to a finding
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum RelatedInfo {
    /// For circular deps: the other module and cycle count
    Cycle {
        other_module: String,
        cycle_count: i64,
    },
    /// For coupling: the list of dependencies
    Dependencies {
        module_name: String,
        fan_out: i64,
        dependencies: Vec<String>,
    },
    /// For drift: the target that was imported
    DriftTarget {
        target_name: String,
        target_file: String,
        boundary_name: String,
        boundary_rule: String,
    },
    /// For dead code: the kind of definition
    DeadCodeKind {
        kind: String,
    },
}

impl Finding {
    pub fn new(
        severity: Severity,
        category: Category,
        file: &str,
        line: Option<i64>,
        name: &str,
        description: &str,
        remediation: &str,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            severity,
            category,
            location: Location {
                file: file.to_string(),
                line,
                name: name.to_string(),
            },
            description: description.to_string(),
            remediation: remediation.to_string(),
            related: None,
        }
    }
}