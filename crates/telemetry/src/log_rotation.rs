//! Log file rotation by size and time.
//!
//! Provides a lightweight log rotator that checks log files and rotates
//! them when they exceed a size threshold or age limit. All logs stay local.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

// ── Configuration ─────────────────────────────────────────────────────

/// Configuration for log file rotation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RotationConfig {
    /// Maximum size in bytes before rotation (default: 10 MB).
    pub max_size_bytes: u64,
    /// Maximum age before rotation (default: 7 days).
    #[serde(with = "duration_secs")]
    pub max_age: Duration,
    /// Maximum number of rotated files to keep (default: 5).
    pub max_files: usize,
}

impl Default for RotationConfig {
    fn default() -> Self {
        Self {
            max_size_bytes: 10 * 1024 * 1024,            // 10 MB
            max_age: Duration::from_secs(7 * 24 * 3600), // 7 days
            max_files: 5,
        }
    }
}

/// Serde helper for Duration as seconds.
mod duration_secs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
        duration.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(Duration::from_secs(secs))
    }
}

// ── Rotation check result ─────────────────────────────────────────────

/// Reason a log file needs rotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotationReason {
    /// File exceeds the maximum size.
    SizeExceeded { current: u64, max: u64 },
    /// File exceeds the maximum age.
    AgeExceeded,
    /// No rotation needed.
    NotNeeded,
}

impl RotationReason {
    /// Whether rotation is needed.
    #[must_use]
    pub fn needs_rotation(&self) -> bool {
        !matches!(self, Self::NotNeeded)
    }
}

// ── LogRotator ────────────────────────────────────────────────────────

/// Manages log file rotation for a given log directory.
pub struct LogRotator {
    log_dir: PathBuf,
    prefix: String,
    config: RotationConfig,
}

impl std::fmt::Debug for LogRotator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogRotator")
            .field("log_dir", &self.log_dir)
            .field("prefix", &self.prefix)
            .field("config", &self.config)
            .finish()
    }
}

impl LogRotator {
    /// Create a new rotator for log files in `log_dir` with the given prefix.
    #[must_use]
    pub fn new(log_dir: PathBuf, prefix: &str, config: RotationConfig) -> Self {
        Self {
            log_dir,
            prefix: prefix.to_string(),
            config,
        }
    }

    /// Default log directory: `~/.crab/logs/`.
    #[must_use]
    pub fn default_log_dir() -> PathBuf {
        crab_common::path::home_dir().join(".crab").join("logs")
    }

    /// Check whether the current log file needs rotation.
    #[must_use]
    pub fn check_rotation(&self, file_path: &Path) -> RotationReason {
        let Ok(metadata) = std::fs::metadata(file_path) else {
            return RotationReason::NotNeeded; // file doesn't exist
        };

        // Check size
        let size = metadata.len();
        if size >= self.config.max_size_bytes {
            return RotationReason::SizeExceeded {
                current: size,
                max: self.config.max_size_bytes,
            };
        }

        // Check age
        if let Ok(modified) = metadata.modified()
            && let Ok(age) = SystemTime::now().duration_since(modified)
            && age >= self.config.max_age
        {
            return RotationReason::AgeExceeded;
        }

        RotationReason::NotNeeded
    }

    /// Rotate a log file: rename `file.log` → `file.log.1`, shift existing
    /// rotated files up, and remove files beyond `max_files`.
    pub fn rotate(&self, file_path: &Path) -> crab_common::Result<()> {
        if !file_path.exists() {
            return Ok(());
        }

        let path_str = file_path.to_string_lossy().to_string();

        // Remove the oldest file if it exceeds max_files
        let oldest = format!("{path_str}.{}", self.config.max_files);
        let _ = std::fs::remove_file(&oldest);

        // Shift existing rotated files up: .4 → .5, .3 → .4, etc.
        for i in (1..self.config.max_files).rev() {
            let from = format!("{path_str}.{i}");
            let to = format!("{path_str}.{}", i + 1);
            if Path::new(&from).exists() {
                std::fs::rename(&from, &to).map_err(|e| {
                    crab_common::Error::Other(format!("failed to rotate {from} → {to}: {e}"))
                })?;
            }
        }

        // Rename current file to .1
        let rotated = format!("{path_str}.1");
        std::fs::rename(file_path, &rotated).map_err(|e| {
            crab_common::Error::Other(format!(
                "failed to rotate {} → {rotated}: {e}",
                file_path.display()
            ))
        })?;

        Ok(())
    }

    /// Check and rotate if needed. Returns whether rotation occurred.
    pub fn check_and_rotate(&self, file_path: &Path) -> crab_common::Result<bool> {
        let reason = self.check_rotation(file_path);
        if reason.needs_rotation() {
            self.rotate(file_path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// List existing rotated files for a given log file, sorted by index.
    #[must_use]
    pub fn list_rotated_files(&self, file_path: &Path) -> Vec<PathBuf> {
        let path_str = file_path.to_string_lossy().to_string();
        let mut files = Vec::new();
        for i in 1..=self.config.max_files {
            let rotated = PathBuf::from(format!("{path_str}.{i}"));
            if rotated.exists() {
                files.push(rotated);
            }
        }
        files
    }

    /// Clean up old rotated files beyond `max_files`.
    pub fn cleanup(&self, file_path: &Path) -> crab_common::Result<usize> {
        let path_str = file_path.to_string_lossy().to_string();
        let mut removed = 0;
        // Check for files beyond max_files
        for i in (self.config.max_files + 1)..=(self.config.max_files + 10) {
            let old = PathBuf::from(format!("{path_str}.{i}"));
            if old.exists() {
                std::fs::remove_file(&old).map_err(|e| {
                    crab_common::Error::Other(format!("failed to remove {}: {e}", old.display()))
                })?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Get the log directory.
    #[must_use]
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// Get the file prefix.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Get the rotation config.
    #[must_use]
    pub fn config(&self) -> &RotationConfig {
        &self.config
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crab-log-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup_dir(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── RotationConfig ────────────────────────────────────────────────

    #[test]
    fn rotation_config_default() {
        let config = RotationConfig::default();
        assert_eq!(config.max_size_bytes, 10 * 1024 * 1024);
        assert_eq!(config.max_age, Duration::from_secs(7 * 24 * 3600));
        assert_eq!(config.max_files, 5);
    }

    #[test]
    fn rotation_config_serde_roundtrip() {
        let config = RotationConfig {
            max_size_bytes: 5_000_000,
            max_age: Duration::from_secs(3600),
            max_files: 3,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: RotationConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, parsed);
    }

    // ── RotationReason ────────────────────────────────────────────────

    #[test]
    fn rotation_reason_needs_rotation() {
        assert!(
            RotationReason::SizeExceeded {
                current: 100,
                max: 50
            }
            .needs_rotation()
        );
        assert!(RotationReason::AgeExceeded.needs_rotation());
        assert!(!RotationReason::NotNeeded.needs_rotation());
    }

    // ── LogRotator ────────────────────────────────────────────────────

    #[test]
    fn check_nonexistent_file_not_needed() {
        let dir = temp_log_dir();
        let rotator = LogRotator::new(dir.clone(), "crab", RotationConfig::default());
        let reason = rotator.check_rotation(&dir.join("nonexistent.log"));
        assert_eq!(reason, RotationReason::NotNeeded);
        cleanup_dir(&dir);
    }

    #[test]
    fn check_small_file_not_needed() {
        let dir = temp_log_dir();
        let file = dir.join("small.log");
        std::fs::write(&file, "small log content").unwrap();

        let rotator = LogRotator::new(dir.clone(), "crab", RotationConfig::default());
        let reason = rotator.check_rotation(&file);
        assert_eq!(reason, RotationReason::NotNeeded);
        cleanup_dir(&dir);
    }

    #[test]
    fn check_large_file_size_exceeded() {
        let dir = temp_log_dir();
        let file = dir.join("large.log");
        // Write a file larger than 100 bytes threshold
        std::fs::write(&file, "x".repeat(200)).unwrap();

        let config = RotationConfig {
            max_size_bytes: 100,
            ..Default::default()
        };
        let rotator = LogRotator::new(dir.clone(), "crab", config);
        let reason = rotator.check_rotation(&file);
        assert!(matches!(reason, RotationReason::SizeExceeded { .. }));
        cleanup_dir(&dir);
    }

    #[test]
    fn rotate_creates_numbered_files() {
        let dir = temp_log_dir();
        let file = dir.join("app.log");
        std::fs::write(&file, "log content v1").unwrap();

        let rotator = LogRotator::new(dir.clone(), "app", RotationConfig::default());
        rotator.rotate(&file).unwrap();

        // Original file should be gone, .1 should exist
        assert!(!file.exists());
        let rotated = dir.join("app.log.1");
        assert!(rotated.exists());
        assert_eq!(std::fs::read_to_string(&rotated).unwrap(), "log content v1");
        cleanup_dir(&dir);
    }

    #[test]
    fn rotate_shifts_existing_files() {
        let dir = temp_log_dir();
        let file = dir.join("app.log");

        // Create existing rotated files
        std::fs::write(dir.join("app.log.1"), "old-1").unwrap();
        std::fs::write(dir.join("app.log.2"), "old-2").unwrap();
        std::fs::write(&file, "current").unwrap();

        let rotator = LogRotator::new(dir.clone(), "app", RotationConfig::default());
        rotator.rotate(&file).unwrap();

        // current → .1, old .1 → .2, old .2 → .3
        assert!(!file.exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("app.log.1")).unwrap(),
            "current"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.log.2")).unwrap(),
            "old-1"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.log.3")).unwrap(),
            "old-2"
        );
        cleanup_dir(&dir);
    }

    #[test]
    fn rotate_removes_oldest_beyond_max() {
        let dir = temp_log_dir();
        let file = dir.join("app.log");
        let config = RotationConfig {
            max_files: 2,
            ..Default::default()
        };

        // Create .1 and .2 (at max)
        std::fs::write(dir.join("app.log.1"), "old-1").unwrap();
        std::fs::write(dir.join("app.log.2"), "old-2").unwrap();
        std::fs::write(&file, "current").unwrap();

        let rotator = LogRotator::new(dir.clone(), "app", config);
        rotator.rotate(&file).unwrap();

        // .2 (old oldest) should be gone, current → .1, old .1 → .2
        assert!(dir.join("app.log.1").exists());
        assert!(dir.join("app.log.2").exists());
        // .2 should now contain what was .1 (old-1)
        assert_eq!(
            std::fs::read_to_string(dir.join("app.log.2")).unwrap(),
            "old-1"
        );
        cleanup_dir(&dir);
    }

    #[test]
    fn rotate_nonexistent_is_noop() {
        let dir = temp_log_dir();
        let rotator = LogRotator::new(dir.clone(), "app", RotationConfig::default());
        assert!(rotator.rotate(&dir.join("nonexistent.log")).is_ok());
        cleanup_dir(&dir);
    }

    #[test]
    fn check_and_rotate_small_file_no_rotation() {
        let dir = temp_log_dir();
        let file = dir.join("small.log");
        std::fs::write(&file, "small").unwrap();

        let rotator = LogRotator::new(dir.clone(), "app", RotationConfig::default());
        let rotated = rotator.check_and_rotate(&file).unwrap();
        assert!(!rotated);
        assert!(file.exists());
        cleanup_dir(&dir);
    }

    #[test]
    fn check_and_rotate_large_file_rotates() {
        let dir = temp_log_dir();
        let file = dir.join("big.log");
        std::fs::write(&file, "x".repeat(200)).unwrap();

        let config = RotationConfig {
            max_size_bytes: 100,
            ..Default::default()
        };
        let rotator = LogRotator::new(dir.clone(), "app", config);
        let rotated = rotator.check_and_rotate(&file).unwrap();
        assert!(rotated);
        assert!(!file.exists());
        assert!(dir.join("big.log.1").exists());
        cleanup_dir(&dir);
    }

    #[test]
    fn list_rotated_files() {
        let dir = temp_log_dir();
        let file = dir.join("app.log");
        std::fs::write(dir.join("app.log.1"), "1").unwrap();
        std::fs::write(dir.join("app.log.3"), "3").unwrap();
        // Note: .2 is missing (gap)

        let rotator = LogRotator::new(dir.clone(), "app", RotationConfig::default());
        let files = rotator.list_rotated_files(&file);
        assert_eq!(files.len(), 2);
        cleanup_dir(&dir);
    }

    #[test]
    fn cleanup_removes_extra_files() {
        let dir = temp_log_dir();
        let file = dir.join("app.log");
        let config = RotationConfig {
            max_files: 2,
            ..Default::default()
        };

        // Create files beyond max
        std::fs::write(dir.join("app.log.3"), "extra-3").unwrap();
        std::fs::write(dir.join("app.log.4"), "extra-4").unwrap();

        let rotator = LogRotator::new(dir.clone(), "app", config);
        let removed = rotator.cleanup(&file).unwrap();
        assert_eq!(removed, 2);
        assert!(!dir.join("app.log.3").exists());
        assert!(!dir.join("app.log.4").exists());
        cleanup_dir(&dir);
    }

    // ── Path helpers ──────────────────────────────────────────────────

    #[test]
    fn default_log_dir_under_crab() {
        let dir = LogRotator::default_log_dir();
        assert!(dir.to_string_lossy().contains(".crab"));
        assert!(dir.ends_with("logs"));
    }

    // ── Accessors ─────────────────────────────────────────────────────

    #[test]
    fn rotator_accessors() {
        let dir = PathBuf::from("/tmp/logs");
        let config = RotationConfig::default();
        let rotator = LogRotator::new(dir.clone(), "crab", config.clone());
        assert_eq!(rotator.log_dir(), dir);
        assert_eq!(rotator.prefix(), "crab");
        assert_eq!(rotator.config(), &config);
    }

    #[test]
    fn rotator_debug() {
        let rotator = LogRotator::new(PathBuf::from("/tmp"), "crab", RotationConfig::default());
        let debug = format!("{rotator:?}");
        assert!(debug.contains("LogRotator"));
    }
}
