//! Tool usage pattern detection and next-tool prediction from historical
//! invocation sequences.

use std::collections::HashMap;

use crate::tool_analytics::ToolUsageRecord;

// ── Data model ─────────────────────────────────────────────────────────

/// A detected sequential pattern of tool calls.
#[derive(Debug, Clone)]
pub struct ToolPattern {
    /// Ordered tool-name sequence (e.g., `["read", "edit", "bash"]`).
    pub sequence: Vec<String>,
    /// How often this sequence appeared.
    pub frequency: u32,
    /// Average success rate across all occurrences of this pattern.
    pub avg_success_rate: f64,
}

// ── Pattern detection ──────────────────────────────────────────────────

/// Detects recurring tool-call sequences (n-grams) from a history of records.
#[derive(Debug, Clone)]
pub struct PatternDetector {
    /// Minimum sequence length to consider (default 2).
    pub min_len: usize,
    /// Maximum sequence length to consider (default 4).
    pub max_len: usize,
}

impl PatternDetector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            min_len: 2,
            max_len: 4,
        }
    }

    /// Extract tool-name sequence from records (ordered by timestamp).
    fn extract_tool_names(history: &[ToolUsageRecord]) -> Vec<(String, bool)> {
        let mut sorted: Vec<&ToolUsageRecord> = history.iter().collect();
        sorted.sort_by_key(|r| r.timestamp_ms);
        sorted.iter().map(|r| (r.tool_name.clone(), r.success)).collect()
    }

    /// Detect all patterns with frequency >= `min_frequency`.
    #[must_use]
    pub fn detect_patterns(
        &self,
        history: &[ToolUsageRecord],
        min_frequency: u32,
    ) -> Vec<ToolPattern> {
        if history.len() < self.min_len {
            return Vec::new();
        }

        let names = Self::extract_tool_names(history);
        let mut patterns: Vec<ToolPattern> = Vec::new();

        for n in self.min_len..=self.max_len {
            if n > names.len() {
                break;
            }
            // Count n-gram frequencies and track success rates.
            let mut counts: HashMap<Vec<String>, (u32, u32, u32)> = HashMap::new(); // (freq, successes, total)
            for window in names.windows(n) {
                let key: Vec<String> = window.iter().map(|(name, _)| name.clone()).collect();
                let entry = counts.entry(key).or_insert((0, 0, 0));
                entry.0 += 1;
                for (_, success) in window {
                    entry.2 += 1;
                    if *success {
                        entry.1 += 1;
                    }
                }
            }

            for (seq, (freq, successes, total)) in counts {
                if freq >= min_frequency {
                    let avg_success_rate = if total > 0 {
                        successes as f64 / total as f64
                    } else {
                        0.0
                    };
                    patterns.push(ToolPattern {
                        sequence: seq,
                        frequency: freq,
                        avg_success_rate,
                    });
                }
            }
        }

        // Sort by frequency descending, then by sequence length descending.
        patterns.sort_by(|a, b| {
            b.frequency
                .cmp(&a.frequency)
                .then_with(|| b.sequence.len().cmp(&a.sequence.len()))
        });
        patterns
    }
}

impl Default for PatternDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ── Next-tool prediction ───────────────────────────────────────────────

/// Detect patterns from `history` and predict the most likely next tool
/// given the `recent_tools` suffix.
///
/// Returns `None` if no matching pattern is found.
#[must_use]
pub fn detect_patterns(
    history: &[ToolUsageRecord],
    min_frequency: u32,
) -> Vec<ToolPattern> {
    PatternDetector::new().detect_patterns(history, min_frequency)
}

/// Suggest the next tool based on recent tool sequence and historical patterns.
///
/// Scans detected patterns for a match where the pattern prefix matches the
/// end of `recent_tools`, then returns the predicted next tool.
#[must_use]
pub fn suggest_next_tool(
    history: &[ToolUsageRecord],
    recent_tools: &[String],
) -> Option<String> {
    suggest_next_tool_with_min_freq(history, recent_tools, 2)
}

/// Like `suggest_next_tool` but with configurable minimum frequency.
#[must_use]
pub fn suggest_next_tool_with_min_freq(
    history: &[ToolUsageRecord],
    recent_tools: &[String],
    min_frequency: u32,
) -> Option<String> {
    if recent_tools.is_empty() {
        return None;
    }

    let patterns = detect_patterns(history, min_frequency);

    // Try matching longer prefixes first (more specific).
    for pattern in &patterns {
        let prefix_len = pattern.sequence.len() - 1;
        if prefix_len == 0 || prefix_len > recent_tools.len() {
            continue;
        }
        let pattern_prefix = &pattern.sequence[..prefix_len];
        let recent_suffix = &recent_tools[recent_tools.len() - prefix_len..];
        if pattern_prefix == recent_suffix {
            return Some(pattern.sequence.last().unwrap().clone());
        }
    }
    None
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &str, ts: u64, success: bool) -> ToolUsageRecord {
        ToolUsageRecord {
            tool_name: name.to_string(),
            timestamp_ms: ts,
            duration_ms: 10,
            success,
            input_size: 0,
            output_size: 0,
        }
    }

    #[test]
    fn empty_history() {
        let patterns = detect_patterns(&[], 1);
        assert!(patterns.is_empty());
    }

    #[test]
    fn single_record_no_patterns() {
        let history = vec![rec("read", 0, true)];
        let patterns = detect_patterns(&history, 1);
        assert!(patterns.is_empty());
    }

    #[test]
    fn detects_bigram_pattern() {
        // read → edit occurs 3 times
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, true),
            rec("bash", 20, true),
            rec("read", 30, true),
            rec("edit", 40, true),
            rec("bash", 50, true),
            rec("read", 60, true),
            rec("edit", 70, true),
        ];
        let patterns = detect_patterns(&history, 2);
        let read_edit = patterns.iter().find(|p| p.sequence == vec!["read", "edit"]);
        assert!(read_edit.is_some());
        assert!(read_edit.unwrap().frequency >= 3);
    }

    #[test]
    fn detects_trigram_pattern() {
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, true),
            rec("bash", 20, true),
            rec("read", 30, true),
            rec("edit", 40, true),
            rec("bash", 50, true),
            rec("read", 60, true),
            rec("edit", 70, true),
            rec("bash", 80, true),
        ];
        let patterns = detect_patterns(&history, 2);
        let trigram = patterns.iter().find(|p| p.sequence == vec!["read", "edit", "bash"]);
        assert!(trigram.is_some());
        assert!(trigram.unwrap().frequency >= 3);
    }

    #[test]
    fn min_frequency_filters() {
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, true),
            rec("read", 20, true),
            rec("edit", 30, true),
        ];
        let patterns = detect_patterns(&history, 5);
        assert!(patterns.is_empty());
    }

    #[test]
    fn success_rate_calculation() {
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, false),
            rec("read", 20, true),
            rec("edit", 30, true),
        ];
        let patterns = detect_patterns(&history, 2);
        let p = patterns.iter().find(|p| p.sequence == vec!["read", "edit"]).unwrap();
        // 4 items total across 2 windows: read(t),edit(f),read(t),edit(t) → 3/4=0.75
        assert!(p.avg_success_rate > 0.7 && p.avg_success_rate < 0.8);
    }

    #[test]
    fn patterns_sorted_by_frequency() {
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, true),
            rec("read", 20, true),
            rec("edit", 30, true),
            rec("read", 40, true),
            rec("edit", 50, true),
            rec("bash", 60, true),
            rec("grep", 70, true),
        ];
        let patterns = detect_patterns(&history, 1);
        for w in patterns.windows(2) {
            assert!(w[0].frequency >= w[1].frequency);
        }
    }

    #[test]
    fn suggest_next_tool_basic() {
        // Pattern: read → edit → bash occurs 3 times
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, true),
            rec("bash", 20, true),
            rec("read", 30, true),
            rec("edit", 40, true),
            rec("bash", 50, true),
            rec("read", 60, true),
            rec("edit", 70, true),
            rec("bash", 80, true),
        ];
        let recent = vec!["read".to_string(), "edit".to_string()];
        let next = suggest_next_tool(&history, &recent);
        assert_eq!(next, Some("bash".to_string()));
    }

    #[test]
    fn suggest_next_tool_empty_recent() {
        let history = vec![rec("read", 0, true), rec("edit", 10, true)];
        let next = suggest_next_tool(&history, &[]);
        assert!(next.is_none());
    }

    #[test]
    fn suggest_next_tool_no_match() {
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, true),
            rec("read", 20, true),
            rec("edit", 30, true),
        ];
        let recent = vec!["bash".to_string(), "grep".to_string()];
        let next = suggest_next_tool(&history, &recent);
        assert!(next.is_none());
    }

    #[test]
    fn suggest_from_bigram() {
        // read → edit occurs twice
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, true),
            rec("bash", 20, true),
            rec("read", 30, true),
            rec("edit", 40, true),
        ];
        let recent = vec!["read".to_string()];
        let next = suggest_next_tool(&history, &recent);
        assert_eq!(next, Some("edit".to_string()));
    }

    #[test]
    fn detector_custom_lengths() {
        let mut d = PatternDetector::new();
        d.min_len = 3;
        d.max_len = 3;
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, true),
            rec("bash", 20, true),
            rec("read", 30, true),
            rec("edit", 40, true),
            rec("bash", 50, true),
        ];
        let patterns = d.detect_patterns(&history, 1);
        // Only trigrams
        for p in &patterns {
            assert_eq!(p.sequence.len(), 3);
        }
    }

    #[test]
    fn detector_default_trait() {
        let d = PatternDetector::default();
        assert_eq!(d.min_len, 2);
        assert_eq!(d.max_len, 4);
    }

    #[test]
    fn timestamp_ordering() {
        // Records out of order should still be sorted correctly
        let history = vec![
            rec("bash", 30, true),
            rec("read", 10, true),
            rec("edit", 20, true),
            rec("read", 40, true),
            rec("edit", 50, true),
            rec("bash", 60, true),
        ];
        // Sorted: read,edit,bash,read,edit,bash → read→edit bigram freq=2
        let patterns = detect_patterns(&history, 2);
        let re = patterns.iter().find(|p| p.sequence == vec!["read", "edit"]);
        assert!(re.is_some());
    }

    #[test]
    fn pattern_with_all_failures() {
        let history = vec![
            rec("read", 0, false),
            rec("edit", 10, false),
            rec("read", 20, false),
            rec("edit", 30, false),
        ];
        let patterns = detect_patterns(&history, 2);
        let p = patterns.iter().find(|p| p.sequence == vec!["read", "edit"]).unwrap();
        assert_eq!(p.avg_success_rate, 0.0);
    }

    #[test]
    fn max_len_respected() {
        let d = PatternDetector { min_len: 2, max_len: 2 };
        let history: Vec<ToolUsageRecord> = (0..10)
            .map(|i| rec("read", i * 10, true))
            .collect();
        let patterns = d.detect_patterns(&history, 1);
        for p in &patterns {
            assert!(p.sequence.len() <= 2);
        }
    }

    #[test]
    fn suggest_with_custom_min_freq() {
        let history = vec![
            rec("read", 0, true),
            rec("edit", 10, true),
            rec("read", 20, true),
            rec("edit", 30, true),
        ];
        // min_freq=1 should match
        let next = suggest_next_tool_with_min_freq(&history, &["read".to_string()], 1);
        assert_eq!(next, Some("edit".to_string()));
        // min_freq=10 should not match
        let next2 = suggest_next_tool_with_min_freq(&history, &["read".to_string()], 10);
        assert!(next2.is_none());
    }
}
