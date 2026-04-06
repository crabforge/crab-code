//! Smart command suggestions based on history, file context, and recent failures.
//!
//! [`CommandSuggester`] generates ranked command suggestions from a combination
//! of strategies: history frequency, file-type inference, and retry of recent
//! failed commands.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::command_history::{CommandHistory, HistoryEntry};

// ── Suggestion ──────────────────────────────────────────────────────

/// A suggested command with confidence and explanation.
#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    /// The suggested command line.
    pub command: String,
    /// Confidence score (0.0–1.0, higher = more confident).
    pub confidence: f64,
    /// Human-readable reason for the suggestion.
    pub reason: String,
}

// ── Suggester ───────────────────────────────────────────────────────

/// Generates command suggestions from history and context.
pub struct CommandSuggester<'a> {
    history: &'a CommandHistory,
}

impl<'a> CommandSuggester<'a> {
    /// Create a suggester backed by the given history.
    #[must_use]
    pub fn new(history: &'a CommandHistory) -> Self {
        Self { history }
    }

    /// Generate suggestions for a partial input in the given working directory.
    ///
    /// Combines multiple strategies and returns up to `max` suggestions sorted
    /// by confidence (highest first).
    #[must_use]
    pub fn suggest(&self, partial: &str, cwd: &Path, max: usize) -> Vec<Suggestion> {
        let mut suggestions = Vec::new();

        suggestions.extend(self.suggest_by_prefix(partial));
        suggestions.extend(self.suggest_by_frequency(partial));
        suggestions.extend(self.suggest_from_failures());
        suggestions.extend(Self::suggest_from_files(cwd));

        // Deduplicate by command, keeping highest confidence
        let mut best: BTreeMap<String, Suggestion> = BTreeMap::new();
        for s in suggestions {
            let entry = best.entry(s.command.clone()).or_insert_with(|| s.clone());
            if s.confidence > entry.confidence {
                *entry = s;
            }
        }

        let mut sorted: Vec<_> = best.into_values().collect();
        sorted.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(max);
        sorted
    }

    /// Suggest commands that start with the partial input (prefix match on history).
    fn suggest_by_prefix(&self, partial: &str) -> Vec<Suggestion> {
        if partial.is_empty() {
            return Vec::new();
        }
        let pat = partial.to_lowercase();
        let mut freq: BTreeMap<String, usize> = BTreeMap::new();

        for entry in self.history.entries() {
            let full = full_command(entry);
            if full.to_lowercase().starts_with(&pat) {
                *freq.entry(full).or_default() += 1;
            }
        }

        let max_freq = freq.values().copied().max().unwrap_or(1).max(1);
        freq.into_iter()
            .map(|(cmd, count)| Suggestion {
                command: cmd,
                #[allow(clippy::cast_precision_loss)]
                confidence: 0.8 * (count as f64 / max_freq as f64),
                reason: "matches history prefix".to_owned(),
            })
            .collect()
    }

    /// Suggest the most frequently used commands from history.
    #[allow(clippy::cast_precision_loss)]
    fn suggest_by_frequency(&self, partial: &str) -> Vec<Suggestion> {
        let mut freq: BTreeMap<String, usize> = BTreeMap::new();
        for entry in self.history.entries() {
            let cmd = entry.command.clone();
            *freq.entry(cmd).or_default() += 1;
        }

        let max_freq = freq.values().copied().max().unwrap_or(1).max(1);
        let pat = partial.to_lowercase();

        freq.into_iter()
            .filter(|(cmd, _)| partial.is_empty() || cmd.to_lowercase().contains(&pat))
            .map(|(cmd, count)| Suggestion {
                command: cmd,
                confidence: 0.5 * (count as f64 / max_freq as f64),
                reason: "frequently used".to_owned(),
            })
            .collect()
    }

    /// Suggest retrying recently failed commands.
    fn suggest_from_failures(&self) -> Vec<Suggestion> {
        self.history
            .failures()
            .into_iter()
            .rev()
            .take(3)
            .enumerate()
            .map(|(i, entry)| {
                let cmd = full_command(entry);
                #[allow(clippy::cast_precision_loss)]
                let confidence = (i as f64).mul_add(-0.1, 0.4);
                Suggestion {
                    command: cmd,
                    confidence,
                    reason: format!("retry failed command (exit code {:?})", entry.exit_code),
                }
            })
            .collect()
    }

    /// Suggest commands based on files present in the working directory.
    fn suggest_from_files(cwd: &Path) -> Vec<Suggestion> {
        let mut suggestions = Vec::new();

        let checks: &[(&str, &str, &str)] = &[
            ("Cargo.toml", "cargo build", "Cargo.toml detected"),
            ("package.json", "npm install", "package.json detected"),
            ("Makefile", "make", "Makefile detected"),
            (
                "requirements.txt",
                "pip install -r requirements.txt",
                "requirements.txt detected",
            ),
            ("go.mod", "go build ./...", "go.mod detected"),
            ("Dockerfile", "docker build .", "Dockerfile detected"),
            (
                "CMakeLists.txt",
                "cmake --build .",
                "CMakeLists.txt detected",
            ),
        ];

        for (file, cmd, reason) in checks {
            if cwd.join(file).exists() {
                suggestions.push(Suggestion {
                    command: (*cmd).to_owned(),
                    confidence: 0.3,
                    reason: (*reason).to_owned(),
                });
            }
        }

        suggestions
    }
}

/// Build the full command string from a history entry.
fn full_command(entry: &HistoryEntry) -> String {
    if entry.args.is_empty() {
        entry.command.clone()
    } else {
        format!("{} {}", entry.command, entry.args.join(" "))
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_entry(cmd: &str, code: i32) -> HistoryEntry {
        HistoryEntry {
            command: cmd.to_owned(),
            args: Vec::new(),
            exit_code: Some(code),
            duration_ms: 100,
            timestamp: 1000,
            cwd: PathBuf::from("/project"),
        }
    }

    fn make_entry_with_args(cmd: &str, args: &[&str], code: i32) -> HistoryEntry {
        HistoryEntry {
            command: cmd.to_owned(),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            exit_code: Some(code),
            duration_ms: 100,
            timestamp: 1000,
            cwd: PathBuf::from("/project"),
        }
    }

    // ── suggest_by_prefix ──────────────────────────────────

    #[test]
    fn prefix_empty_input() {
        let history = CommandHistory::new(100);
        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest_by_prefix("");
        assert!(results.is_empty());
    }

    #[test]
    fn prefix_matches() {
        let mut history = CommandHistory::new(100);
        history.add(make_entry("cargo test", 0));
        history.add(make_entry("cargo build", 0));
        history.add(make_entry("npm install", 0));

        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest_by_prefix("cargo");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn prefix_case_insensitive() {
        let mut history = CommandHistory::new(100);
        history.add(make_entry("Cargo test", 0));

        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest_by_prefix("cargo");
        assert_eq!(results.len(), 1);
    }

    // ── suggest_by_frequency ──────────────────────────────

    #[test]
    fn frequency_ranking() {
        let mut history = CommandHistory::new(100);
        history.add(make_entry("cargo test", 0));
        history.add(make_entry("cargo test", 0));
        history.add(make_entry("cargo test", 0));
        history.add(make_entry("cargo build", 0));

        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest_by_frequency("");
        // cargo test should have higher confidence
        let test_sug = results.iter().find(|s| s.command == "cargo test").unwrap();
        let build_sug = results.iter().find(|s| s.command == "cargo build").unwrap();
        assert!(test_sug.confidence > build_sug.confidence);
    }

    #[test]
    fn frequency_with_filter() {
        let mut history = CommandHistory::new(100);
        history.add(make_entry("cargo test", 0));
        history.add(make_entry("npm install", 0));

        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest_by_frequency("cargo");
        assert_eq!(results.len(), 1);
    }

    // ── suggest_from_failures ───────────────────────────────────

    #[test]
    fn retry_suggests_failures() {
        let mut history = CommandHistory::new(100);
        history.add(make_entry("cargo test", 1));
        history.add(make_entry("cargo build", 0));

        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest_from_failures();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].command, "cargo test");
    }

    #[test]
    fn retry_max_three() {
        let mut history = CommandHistory::new(100);
        for i in 0..5 {
            history.add(make_entry(&format!("fail{i}"), 1));
        }

        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest_from_failures();
        assert_eq!(results.len(), 3);
    }

    // ── suggest_from_files ───────────────────────────────────

    #[test]
    fn file_context_cargo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let results = CommandSuggester::suggest_from_files(dir.path());
        assert!(!results.is_empty());
        assert!(results.iter().any(|s| s.command.contains("cargo")));
    }

    #[test]
    fn file_context_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let results = CommandSuggester::suggest_from_files(dir.path());
        assert!(results.is_empty());
    }

    #[test]
    fn file_context_package_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();

        let results = CommandSuggester::suggest_from_files(dir.path());
        assert!(results.iter().any(|s| s.command.contains("npm")));
    }

    // ── suggest (integration) ───────────────────────────────

    #[test]
    fn suggest_empty_history_empty_dir() {
        let history = CommandHistory::new(100);
        let dir = tempfile::tempdir().unwrap();
        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest("", dir.path(), 10);
        assert!(results.is_empty());
    }

    #[test]
    fn suggest_deduplicates() {
        let mut history = CommandHistory::new(100);
        history.add(make_entry("cargo test", 0));
        history.add(make_entry("cargo test", 0));

        let dir = tempfile::tempdir().unwrap();
        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest("cargo", dir.path(), 10);
        let cargo_test_count = results.iter().filter(|s| s.command == "cargo test").count();
        assert_eq!(cargo_test_count, 1);
    }

    #[test]
    fn suggest_respects_max() {
        let mut history = CommandHistory::new(100);
        for i in 0..20 {
            history.add(make_entry(&format!("cmd{i}"), 0));
        }

        let dir = tempfile::tempdir().unwrap();
        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest("", dir.path(), 5);
        assert!(results.len() <= 5);
    }

    #[test]
    fn suggest_sorted_by_confidence() {
        let mut history = CommandHistory::new(100);
        history.add(make_entry("cargo test", 0));
        history.add(make_entry("cargo test", 0));
        history.add(make_entry("cargo build", 0));

        let dir = tempfile::tempdir().unwrap();
        let suggester = CommandSuggester::new(&history);
        let results = suggester.suggest("cargo", dir.path(), 10);
        for window in results.windows(2) {
            assert!(window[0].confidence >= window[1].confidence);
        }
    }

    // ── full_command ────────────────────────────────────────

    #[test]
    fn full_command_no_args() {
        let entry = make_entry("ls", 0);
        assert_eq!(full_command(&entry), "ls");
    }

    #[test]
    fn full_command_with_args() {
        let entry = make_entry_with_args("cargo", &["test", "--release"], 0);
        assert_eq!(full_command(&entry), "cargo test --release");
    }

    // ── Suggestion serialization ─────────���──────────────────

    #[test]
    fn suggestion_serializes() {
        let s = Suggestion {
            command: "cargo test".to_owned(),
            confidence: 0.85,
            reason: "frequently used".to_owned(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("cargo test"));
        assert!(json.contains("confidence"));
        assert!(json.contains("reason"));
    }
}
