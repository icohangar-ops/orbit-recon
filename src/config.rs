//! Configuration loading for Orbit Recon

use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Top-level configuration
#[derive(Debug, Deserialize, Default, Clone)]
pub struct Config {
    /// Architecture boundary rules
    #[serde(default)]
    pub boundaries: Vec<BoundaryRule>,

    /// Severity thresholds
    #[serde(default)]
    pub thresholds: Thresholds,

    /// Ignore patterns
    #[serde(default)]
    pub ignore: IgnoreConfig,
}

/// A boundary rule defining which imports a module layer may use
#[derive(Debug, Deserialize, Clone)]
pub struct BoundaryRule {
    /// Human-readable name for this boundary
    pub name: String,
    /// Glob pattern matching files in this layer (e.g., "src/domain/**")
    pub pattern: String,
    /// Glob patterns this layer is allowed to import from
    #[serde(default)]
    pub allowed_imports: Vec<String>,
}

/// Thresholds for severity classification
#[derive(Debug, Deserialize, Clone)]
pub struct Thresholds {
    #[serde(default = "default_dead_code_warning")]
    pub dead_code_warning: usize,
    #[serde(default = "default_dead_code_critical")]
    pub dead_code_critical: usize,
    #[serde(default = "default_coupling_warning")]
    pub coupling_warning_fan_out: usize,
    #[serde(default = "default_coupling_critical")]
    pub coupling_critical_fan_out: usize,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            dead_code_warning: default_dead_code_warning(),
            dead_code_critical: default_dead_code_critical(),
            coupling_warning_fan_out: default_coupling_warning(),
            coupling_critical_fan_out: default_coupling_critical(),
        }
    }
}

fn default_dead_code_warning() -> usize {
    20
}
fn default_dead_code_critical() -> usize {
    50
}
fn default_coupling_warning() -> usize {
    8
}
fn default_coupling_critical() -> usize {
    15
}

/// Patterns to ignore for specific checks
#[derive(Debug, Deserialize, Default, Clone)]
pub struct IgnoreConfig {
    #[serde(default)]
    pub dead_code: Vec<String>,
    #[serde(default)]
    pub drift: Vec<String>,
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        let config: Config = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", path.display()))?;
        Ok(config)
    }

    /// Check if a file path matches any ignore pattern for a given category
    pub fn should_ignore(&self, file: &str, category: &str) -> bool {
        let patterns = match category {
            "dead_code" => &self.ignore.dead_code,
            "drift" => &self.ignore.drift,
            _ => return false,
        };

        for pattern in patterns {
            if let Ok(glob) = glob::Pattern::new(pattern) {
                if glob.matches(file) {
                    return true;
                }
            }
        }
        false
    }

    /// Find which boundary rule applies to a given file path
    pub fn matching_boundary(&self, file: &str) -> Option<&BoundaryRule> {
        self.boundaries.iter().find(|b| {
            if let Ok(glob) = glob::Pattern::new(&b.pattern) {
                glob.matches(file)
            } else {
                false
            }
        })
    }

    /// Check if a file is allowed to import from a target file
    pub fn is_import_allowed(&self, source_file: &str, target_file: &str) -> Option<bool> {
        let boundary = self.matching_boundary(source_file)?;

        for allowed in &boundary.allowed_imports {
            if let Ok(glob) = glob::Pattern::new(allowed) {
                if glob.matches(target_file) {
                    return Some(true);
                }
            }
        }

        Some(false)
    }
}

use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;

    fn boundary(name: &str, pattern: &str, allowed: &[&str]) -> BoundaryRule {
        BoundaryRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            allowed_imports: allowed.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn matching_boundary_finds_layer_by_glob() {
        let cfg = Config {
            boundaries: vec![
                boundary("Domain", "src/domain/**", &["src/domain/**"]),
                boundary("App", "src/application/**", &["src/domain/**"]),
            ],
            ..Default::default()
        };

        let b = cfg.matching_boundary("src/domain/user.rs").unwrap();
        assert_eq!(b.name, "Domain");

        let b = cfg.matching_boundary("src/application/service.rs").unwrap();
        assert_eq!(b.name, "App");

        assert!(cfg.matching_boundary("src/presentation/view.rs").is_none());
    }

    #[test]
    fn import_allowed_respects_glob_scope() {
        let cfg = Config {
            boundaries: vec![boundary(
                "Domain",
                "src/domain/**",
                &["src/domain/**", "src/types/**"],
            )],
            ..Default::default()
        };

        // Allowed: domain importing domain and types.
        assert_eq!(
            cfg.is_import_allowed("src/domain/user.rs", "src/domain/role.rs"),
            Some(true)
        );
        assert_eq!(
            cfg.is_import_allowed("src/domain/user.rs", "src/types/id.rs"),
            Some(true)
        );

        // Disallowed: domain importing presentation.
        assert_eq!(
            cfg.is_import_allowed("src/domain/user.rs", "src/presentation/view.rs"),
            Some(false)
        );

        // Source file outside any boundary: None.
        assert_eq!(
            cfg.is_import_allowed("src/other/x.rs", "src/domain/user.rs"),
            None
        );
    }

    #[test]
    fn should_ignore_matches_dead_code_patterns() {
        let cfg = Config {
            ignore: IgnoreConfig {
                dead_code: vec!["src/generated/**".to_string()],
                drift: vec![],
            },
            ..Default::default()
        };

        assert!(cfg.should_ignore("src/generated/proto.rs", "dead_code"));
        assert!(!cfg.should_ignore("src/domain/user.rs", "dead_code"));
        // Unknown category never ignores.
        assert!(!cfg.should_ignore("src/generated/proto.rs", "unknown"));
    }

    #[test]
    fn double_star_matches_nested_dirs() {
        let cfg = Config {
            boundaries: vec![boundary("Domain", "src/domain/**", &[])],
            ..Default::default()
        };
        // `**` matches across path separators (nested dirs).
        assert!(cfg.matching_boundary("src/domain/user.rs").is_some());
        assert!(cfg.matching_boundary("src/domain/sub/user.rs").is_some());
        // A sibling top-level dir does not match.
        assert!(cfg.matching_boundary("src/application/user.rs").is_none());
    }
}