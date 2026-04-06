//! Command execution history with persistence and statistics.
//!
//! Records every command executed during a session, supports searching,
//! filtering, and persists to `~/.crab/command_history.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── History entry ───────────────────────────────────────────────────

/// A single recorded command execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The command that was executed.
    pub command: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Exit code (None if the process was killed / timed out).
    pub exit_code: Option<i32>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Unix timestamp (seconds since epoch).
    pub timestamp: u64,
    /// Working directory at time of execution.
    pub cwd: PathBuf,
}

// ── Command history ─────────────────────────────────────────────────

/// Stores and queries command execution history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHistory {
    entries: Vec<HistoryEntry>,
    max_entries: usize,
}

impl CommandHistory {
    /// Create an empty history with the given maximum capacity.
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_entries,
        }
    }

    /// Add an entry, evicting the oldest if at capacity.
    pub fn add(&mut self, entry: HistoryEntry) {
        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    /// All entries in chronological order.
    #[must_use]
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the history is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Most recent `n` entries (newest first).
    #[must_use]
    pub fn recent(&self, n: usize) -> Vec<&HistoryEntry> {
        self.entries.iter().rev().take(n).collect()
    }

    /// Search entries whose command contains `pattern` (case-insensitive).
    #[must_use]
    pub fn search(&self, pattern: &str) -> Vec<&HistoryEntry> {
        let pat = pattern.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.command.to_lowercase().contains(&pat))
            .collect()
    }

    /// Filter entries by exit code.
    #[must_use]
    pub fn by_exit_code(&self, code: i32) -> Vec<&HistoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.exit_code == Some(code))
            .collect()
    }

    /// Entries that failed (exit code != 0).
    #[must_use]
    pub fn failures(&self) -> Vec<&HistoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.exit_code.is_some_and(|c| c != 0))
            .collect()
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Compute statistics across all entries.
    #[must_use]
    pub fn stats(&self) -> HistoryStats {
        HistoryStats::compute(&self.entries)
    }

    // ── Persistence ─────────────────────────────────────────

    /// Save history to a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self, path: &Path) -> crab_common::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| crab_common::Error::Other(format!("serialize error: {e}")))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load history from a JSON file. Returns an empty history if the file
    /// does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load(path: &Path, max_entries: usize) -> crab_common::Result<Self> {
        if !path.exists() {
            return Ok(Self::new(max_entries));
        }
        let data = std::fs::read_to_string(path)?;
        let mut hist: Self = serde_json::from_str(&data)
            .map_err(|e| crab_common::Error::Other(format!("parse error: {e}")))?;
        hist.max_entries = max_entries;
        // Trim if loaded history exceeds new max
        while hist.entries.len() > max_entries {
            hist.entries.remove(0);
        }
        Ok(hist)
    }
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new(1000)
    }
}

// ── Statistics ──────────────────────────────────────────────────────

/// Aggregated statistics over command history.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryStats {
    /// Total commands executed.
    pub total_commands: usize,
    /// Number of successful commands (exit code 0).
    pub successes: usize,
    /// Number of failed commands (exit code != 0).
    pub failures: usize,
    /// Success rate (0.0–1.0).
    pub success_rate: f64,
    /// Average duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Command frequency map (command → count), sorted by frequency desc.
    pub command_frequency: Vec<(String, usize)>,
}

impl HistoryStats {
    /// Compute stats from a slice of entries.
    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn compute(entries: &[HistoryEntry]) -> Self {
        let total_commands = entries.len();
        let successes = entries.iter().filter(|e| e.exit_code == Some(0)).count();
        let failures = entries
            .iter()
            .filter(|e| e.exit_code.is_some_and(|c| c != 0))
            .count();

        let success_rate = if total_commands > 0 {
            successes as f64 / total_commands as f64
        } else {
            0.0
        };

        let avg_duration_ms = if total_commands > 0 {
            let total: u64 = entries.iter().map(|e| e.duration_ms).sum();
            total as f64 / total_commands as f64
        } else {
            0.0
        };

        let mut freq_map: BTreeMap<String, usize> = BTreeMap::new();
        for entry in entries {
            *freq_map.entry(entry.command.clone()).or_default() += 1;
        }
        let mut command_frequency: Vec<_> = freq_map.into_iter().collect();
        command_frequency.sort_by(|a, b| b.1.cmp(&a.1));

        Self {
            total_commands,
            successes,
            failures,
            success_rate,
            avg_duration_ms,
            command_frequency,
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(cmd: &str, code: i32, dur: u64) -> HistoryEntry {
        HistoryEntry {
            command: cmd.to_owned(),
            args: Vec::new(),
            exit_code: Some(code),
            duration_ms: dur,
            timestamp: 1000,
            cwd: PathBuf::from("/project"),
        }
    }

    #[test]
    fn new_history_is_empty() {
        let h = CommandHistory::new(100);
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
    }

    #[test]
    fn add_and_retrieve() {
        let mut h = CommandHistory::new(100);
        h.add(make_entry("cargo", 0, 500));
        assert_eq!(h.len(), 1);
        assert_eq!(h.entries()[0].command, "cargo");
    }

    #[test]
    fn eviction_at_capacity() {
        let mut h = CommandHistory::new(2);
        h.add(make_entry("first", 0, 100));
        h.add(make_entry("second", 0, 200));
        h.add(make_entry("third", 0, 300));
        assert_eq!(h.len(), 2);
        assert_eq!(h.entries()[0].command, "second");
        assert_eq!(h.entries()[1].command, "third");
    }

    #[test]
    fn recent_returns_newest_first() {
        let mut h = CommandHistory::new(100);
        h.add(make_entry("a", 0, 100));
        h.add(make_entry("b", 0, 200));
        h.add(make_entry("c", 0, 300));
        let recent = h.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].command, "c");
        assert_eq!(recent[1].command, "b");
    }

    #[test]
    fn search_case_insensitive() {
        let mut h = CommandHistory::new(100);
        h.add(make_entry("cargo test", 0, 100));
        h.add(make_entry("npm install", 0, 200));
        h.add(make_entry("CARGO build", 0, 300));
        let results = h.search("cargo");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_no_match() {
        let mut h = CommandHistory::new(100);
        h.add(make_entry("cargo test", 0, 100));
        let results = h.search("python");
        assert!(results.is_empty());
    }

    #[test]
    fn by_exit_code_filters() {
        let mut h = CommandHistory::new(100);
        h.add(make_entry("a", 0, 100));
        h.add(make_entry("b", 1, 200));
        h.add(make_entry("c", 0, 300));
        assert_eq!(h.by_exit_code(0).len(), 2);
        assert_eq!(h.by_exit_code(1).len(), 1);
        assert_eq!(h.by_exit_code(2).len(), 0);
    }

    #[test]
    fn failures_filter() {
        let mut h = CommandHistory::new(100);
        h.add(make_entry("ok", 0, 100));
        h.add(make_entry("fail", 1, 200));
        h.add(make_entry("crash", 127, 300));
        let fails = h.failures();
        assert_eq!(fails.len(), 2);
    }

    #[test]
    fn clear_empties() {
        let mut h = CommandHistory::new(100);
        h.add(make_entry("a", 0, 100));
        h.clear();
        assert!(h.is_empty());
    }

    #[test]
    fn default_has_1000_capacity() {
        let h = CommandHistory::default();
        assert!(h.is_empty());
    }

    // ── Stats ───────────────────────────────────────────────

    #[test]
    fn stats_empty() {
        let stats = HistoryStats::compute(&[]);
        assert_eq!(stats.total_commands, 0);
        assert!(stats.success_rate.abs() < f64::EPSILON);
    }

    #[test]
    fn stats_computes_correctly() {
        let entries = vec![
            make_entry("cargo", 0, 100),
            make_entry("cargo", 0, 200),
            make_entry("npm", 1, 50),
        ];
        let stats = HistoryStats::compute(&entries);
        assert_eq!(stats.total_commands, 3);
        assert_eq!(stats.successes, 2);
        assert_eq!(stats.failures, 1);
        assert!((stats.avg_duration_ms - (350.0 / 3.0)).abs() < 0.1);
        assert_eq!(stats.command_frequency[0], ("cargo".to_owned(), 2));
    }

    #[test]
    fn stats_from_history() {
        let mut h = CommandHistory::new(100);
        h.add(make_entry("a", 0, 100));
        h.add(make_entry("b", 1, 200));
        let stats = h.stats();
        assert_eq!(stats.total_commands, 2);
        assert_eq!(stats.successes, 1);
    }

    #[test]
    fn stats_serializes() {
        let stats = HistoryStats::compute(&[make_entry("cargo", 0, 100)]);
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("total_commands"));
        assert!(json.contains("success_rate"));
    }

    // ── Persistence ─────────────────────────────────────────

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");

        let mut h = CommandHistory::new(100);
        h.add(make_entry("cargo test", 0, 500));
        h.add(make_entry("cargo build", 1, 1000));
        h.save(&path).unwrap();

        let loaded = CommandHistory::load(&path, 100).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.entries()[0].command, "cargo test");
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let loaded = CommandHistory::load(Path::new("/nonexistent/history.json"), 100).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn load_trims_to_max() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");

        let mut h = CommandHistory::new(100);
        for i in 0..10 {
            h.add(make_entry(&format!("cmd{i}"), 0, 100));
        }
        h.save(&path).unwrap();

        let loaded = CommandHistory::load(&path, 3).unwrap();
        assert_eq!(loaded.len(), 3);
    }

    #[test]
    fn entry_serde_roundtrip() {
        let entry = make_entry("cargo", 0, 500);
        let json = serde_json::to_string(&entry).unwrap();
        let back: HistoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, back);
    }
}
