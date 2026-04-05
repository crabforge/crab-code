//! Symlink safety: detect and prevent symlink escapes out of a project directory.
//!
//! When tools read or write files, symlinks could be used to access files
//! outside the allowed project boundary. This module provides utilities to
//! resolve symlinks and verify that the resolved target stays within the
//! permitted directory.

use std::path::{Path, PathBuf};

// ── Public API ────────────────────────────────────────────────────────

/// Check whether `target` resolves to a path within `boundary`.
///
/// Resolves all symlinks in `target` using [`std::fs::canonicalize`] and
/// verifies the result starts with the canonicalized `boundary`.
///
/// # Errors
///
/// Returns an error if either path cannot be canonicalized (e.g., does
/// not exist) or if the resolved target escapes the boundary.
pub fn check_symlink_safety(target: &Path, boundary: &Path) -> crab_common::Result<PathBuf> {
    let resolved_target = std::fs::canonicalize(target).map_err(|e| {
        crab_common::Error::Other(format!("cannot resolve path {}: {e}", target.display()))
    })?;

    let resolved_boundary = std::fs::canonicalize(boundary).map_err(|e| {
        crab_common::Error::Other(format!(
            "cannot resolve boundary {}: {e}",
            boundary.display()
        ))
    })?;

    if resolved_target.starts_with(&resolved_boundary) {
        Ok(resolved_target)
    } else {
        Err(crab_common::Error::Other(format!(
            "symlink escape detected: {} resolves to {} which is outside {}",
            target.display(),
            resolved_target.display(),
            resolved_boundary.display(),
        )))
    }
}

/// Check whether `target` is a symlink.
#[must_use]
pub fn is_symlink(target: &Path) -> bool {
    target.symlink_metadata().is_ok_and(|m| m.is_symlink())
}

/// Resolve a path fully, following all symlinks.
///
/// Unlike [`std::fs::canonicalize`], this returns a friendly error.
///
/// # Errors
///
/// Returns an error if the path cannot be resolved.
pub fn resolve(target: &Path) -> crab_common::Result<PathBuf> {
    std::fs::canonicalize(target).map_err(|e| {
        crab_common::Error::Other(format!("cannot resolve path {}: {e}", target.display()))
    })
}

/// Check a path without requiring it to exist yet. Resolves the parent
/// directory and verifies the result is within `boundary`.
///
/// Useful for write operations where the target file may not exist.
///
/// # Errors
///
/// Returns an error if the parent cannot be resolved or the path escapes.
pub fn check_parent_safety(target: &Path, boundary: &Path) -> crab_common::Result<PathBuf> {
    let parent = target.parent().ok_or_else(|| {
        crab_common::Error::Other(format!("path has no parent: {}", target.display()))
    })?;

    let resolved_parent = std::fs::canonicalize(parent).map_err(|e| {
        crab_common::Error::Other(format!("cannot resolve parent {}: {e}", parent.display()))
    })?;

    let resolved_boundary = std::fs::canonicalize(boundary).map_err(|e| {
        crab_common::Error::Other(format!(
            "cannot resolve boundary {}: {e}",
            boundary.display()
        ))
    })?;

    if resolved_parent.starts_with(&resolved_boundary) {
        // Return the full resolved path (parent + filename)
        let filename = target
            .file_name()
            .ok_or_else(|| crab_common::Error::Other("path has no filename".into()))?;
        Ok(resolved_parent.join(filename))
    } else {
        Err(crab_common::Error::Other(format!(
            "symlink escape detected: parent of {} resolves to {} which is outside {}",
            target.display(),
            resolved_parent.display(),
            resolved_boundary.display(),
        )))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn safe_path_within_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("safe.txt");
        fs::write(&file, "ok").unwrap();

        let result = check_symlink_safety(&file, dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn nonexistent_target_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = check_symlink_safety(&dir.path().join("nope.txt"), dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot resolve path")
        );
    }

    #[test]
    fn nonexistent_boundary_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "ok").unwrap();
        let result = check_symlink_safety(&file, Path::new("/nonexistent/boundary"));
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_within_boundary_is_safe() {
        let dir = tempfile::tempdir().unwrap();
        let real_file = dir.path().join("real.txt");
        fs::write(&real_file, "data").unwrap();

        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&real_file, &link).unwrap();

        let result = check_symlink_safety(&link, dir.path());
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_detected() {
        let boundary = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, "secret").unwrap();

        let link = boundary.path().join("escape.txt");
        std::os::unix::fs::symlink(&outside_file, &link).unwrap();

        let result = check_symlink_safety(&link, boundary.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("symlink escape"));
    }

    #[test]
    fn is_symlink_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("regular.txt");
        fs::write(&file, "data").unwrap();
        assert!(!is_symlink(&file));
    }

    #[test]
    fn is_symlink_nonexistent() {
        assert!(!is_symlink(Path::new("/does/not/exist")));
    }

    #[cfg(unix)]
    #[test]
    fn is_symlink_detects_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.txt");
        fs::write(&real, "data").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(is_symlink(&link));
    }

    #[test]
    fn resolve_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "data").unwrap();
        let resolved = resolve(&file).unwrap();
        assert!(resolved.is_absolute());
    }

    #[test]
    fn resolve_nonexistent_errors() {
        let result = resolve(Path::new("/does/not/exist/at/all"));
        assert!(result.is_err());
    }

    #[test]
    fn check_parent_safety_new_file() {
        let dir = tempfile::tempdir().unwrap();
        // File doesn't exist yet, but parent does
        let new_file = dir.path().join("new_file.txt");
        let result = check_parent_safety(&new_file, dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.to_string_lossy().contains("new_file.txt"));
    }

    #[test]
    fn check_parent_safety_nonexistent_parent_errors() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("nonexistent_dir").join("file.txt");
        let result = check_parent_safety(&deep, dir.path());
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn check_parent_safety_symlink_escape() {
        let boundary = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        // Create a symlink inside boundary that points to outside
        let link_dir = boundary.path().join("escape_dir");
        std::os::unix::fs::symlink(outside.path(), &link_dir).unwrap();

        let target = link_dir.join("file.txt");
        let result = check_parent_safety(&target, boundary.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("symlink escape"));
    }
}
