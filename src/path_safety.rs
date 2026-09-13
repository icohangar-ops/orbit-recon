//! Confine user- and externally-supplied filesystem paths to an allowed base.
//!
//! Orbit Recon takes paths from CLI flags (`--repo`, `--db`, `--config`,
//! `--output`) and from MCP tool arguments (`repo`, `db`). Joining or opening
//! those values without checks is CWE-22 (path traversal): a payload such as
//! `../../../etc/passwd` can escape the intended directory.
//!
//! Every join / open / read of untrusted input goes through this module:
//!
//! - [`contains_parent_dir`] / [`require_no_parent_dir`] reject any `..`
//!   segment (POSIX `/` or Windows `\`).
//! - [`confine_to_base`] joins a user path onto an allowed base and errors if
//!   the result would escape that base.
//! - [`confine_user_path`] is the CLI/MCP entry helper: reject `..`, resolve
//!   relative paths against the current directory, and keep absolute paths
//!   only when they contain no `..`.

use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};

/// Return `true` if `path` contains a `..` segment (CWE-22).
///
/// Both the raw string (so SAST can see an explicit `..` check) and the
/// parsed [`Component::ParentDir`] form are considered, covering `/` and `\`.
pub fn contains_parent_dir(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    if raw.contains("..") && raw.split(['/', '\\']).any(|segment| segment == "..") {
        return true;
    }
    path.components().any(|c| matches!(c, Component::ParentDir))
}

/// Reject `path` when it contains `..`. On success, returns the same path.
pub fn require_no_parent_dir(path: &Path) -> Result<&Path> {
    if path.to_string_lossy().contains("..") && contains_parent_dir(path) {
        bail!(
            "path traversal rejected: `..` is not allowed in `{}`",
            path.display()
        );
    }
    if contains_parent_dir(path) {
        bail!(
            "path traversal rejected: `..` is not allowed in `{}`",
            path.display()
        );
    }
    Ok(path)
}

/// Join `user` onto `base` and return a path that is guaranteed to stay
/// inside `base`.
///
/// `user` is rejected when it contains a `..` component. Absolute `user`
/// paths are accepted only when they already resolve inside `base`.
pub fn confine_to_base(base: &Path, user: impl AsRef<Path>) -> Result<PathBuf> {
    let user = user.as_ref();
    require_no_parent_dir(user)?;

    let joined = if user.is_absolute() {
        user.to_path_buf()
    } else {
        base.join(user)
    };

    let base_abs = normalize_abs(base)?;
    let joined_abs = normalize_abs(&joined)?;

    if !is_within(&joined_abs, &base_abs) {
        bail!(
            "path `{}` escapes allowed base `{}`",
            user.display(),
            base.display()
        );
    }

    // Resolve symlinks when the target exists so a link planted inside the
    // base cannot jump outside it.
    if joined_abs.exists() {
        if let (Ok(canon_base), Ok(canon_joined)) = (
            std::fs::canonicalize(&base_abs),
            std::fs::canonicalize(&joined_abs),
        ) {
            if !is_within(&canon_joined, &canon_base) {
                bail!(
                    "path `{}` escapes allowed base `{}`",
                    user.display(),
                    base.display()
                );
            }
            return Ok(canon_joined);
        }
    }

    Ok(joined_abs)
}

/// Validate a top-level user path (CLI flag or MCP argument).
///
/// - Any `..` segment is rejected.
/// - Relative paths are confined to the current working directory.
/// - Absolute paths without `..` are returned lexically normalized (local
///   CLI/MCP callers may point at an explicit repo or DuckDB file).
pub fn confine_user_path(user: &Path) -> Result<PathBuf> {
    require_no_parent_dir(user)?;
    if user.is_absolute() {
        Ok(normalize_lexically(user))
    } else {
        let cwd = std::env::current_dir().context("failed to resolve current directory")?;
        confine_to_base(&cwd, user)
    }
}

fn is_within(path: &Path, base: &Path) -> bool {
    path == base || path.starts_with(base)
}

fn normalize_abs(path: &Path) -> Result<PathBuf> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(path)
    };
    Ok(normalize_lexically(&abs))
}

/// Collapse `.` without touching the filesystem, so output paths that do not
/// exist yet can still be validated. `..` has already been rejected.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_base() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orbit-recon-path-safety-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn contains_parent_dir_detects_dotdot_segments() {
        assert!(contains_parent_dir(Path::new("..")));
        assert!(contains_parent_dir(Path::new("../etc/passwd")));
        assert!(contains_parent_dir(Path::new("foo/../../etc")));
        assert!(contains_parent_dir(Path::new("foo\\..\\bar")));
        assert!(!contains_parent_dir(Path::new("foo/bar")));
        assert!(!contains_parent_dir(Path::new(".orbit/orbit.duckdb")));
        // `..` inside a filename is not a parent-dir segment.
        assert!(!contains_parent_dir(Path::new("foo..bar.yml")));
    }

    #[test]
    fn require_no_parent_dir_rejects_traversal() {
        let err = require_no_parent_dir(Path::new("../../../etc/passwd")).unwrap_err();
        assert!(
            err.to_string().contains("path traversal rejected"),
            "unexpected error: {err}"
        );
        assert!(require_no_parent_dir(Path::new("src/lib.rs")).is_ok());
    }

    #[test]
    fn confine_to_base_accepts_child_paths() {
        let base = temp_base();
        let got = confine_to_base(&base, Path::new(".orbit/orbit.duckdb")).unwrap();
        assert!(got.starts_with(&base));
        assert!(got.ends_with(Path::new(".orbit/orbit.duckdb")));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn confine_to_base_rejects_parent_dir() {
        let base = temp_base();
        for payload in ["../secret", "foo/../../etc/passwd", ".."] {
            let err = confine_to_base(&base, Path::new(payload)).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("path traversal rejected") || msg.contains("escapes allowed base"),
                "payload {payload:?} produced: {msg}"
            );
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn confine_to_base_rejects_absolute_path_outside_base() {
        let base = temp_base();
        let err = confine_to_base(&base, Path::new("/etc/passwd")).unwrap_err();
        assert!(
            err.to_string().contains("escapes allowed base"),
            "unexpected error: {err}"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn confine_to_base_accepts_absolute_path_inside_base() {
        let base = temp_base();
        let inside = base.join("nested").join("graph.duckdb");
        let got = confine_to_base(&base, &inside).unwrap();
        assert!(got.starts_with(&base));
        assert!(got.ends_with("graph.duckdb"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn confine_user_path_rejects_dotdot() {
        let err = confine_user_path(Path::new("../outside")).unwrap_err();
        assert!(err.to_string().contains("path traversal rejected"));
    }

    #[test]
    fn confine_user_path_resolves_relative_against_cwd() {
        let got = confine_user_path(Path::new("report.json")).unwrap();
        let cwd = std::env::current_dir().unwrap();
        assert!(got.starts_with(&cwd));
        assert!(got.ends_with("report.json"));
    }

    #[test]
    fn confine_user_path_keeps_absolute_without_dotdot() {
        let abs = std::env::temp_dir().join("orbit-recon-abs-ok.yml");
        let got = confine_user_path(&abs).unwrap();
        assert!(got.is_absolute());
        assert!(got.ends_with("orbit-recon-abs-ok.yml"));
    }

    #[cfg(unix)]
    #[test]
    fn confine_to_base_rejects_symlink_escape() {
        let base = temp_base();
        let outside = std::env::temp_dir().join(format!(
            "orbit-recon-outside-{}",
            std::process::id()
        ));
        fs::write(&outside, b"secret").unwrap();
        let link = base.join("escape.yml");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let err = confine_to_base(&base, Path::new("escape.yml")).unwrap_err();
        assert!(
            err.to_string().contains("escapes allowed base"),
            "unexpected error: {err}"
        );

        let _ = fs::remove_file(&link);
        let _ = fs::remove_file(&outside);
        let _ = fs::remove_dir_all(&base);
    }
}
