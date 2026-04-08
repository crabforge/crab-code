//! Per-session and per-model token usage aggregation.
//!
//! Collects input/output token counts and request counts, broken down by
//! model. Used for cost estimation, session summaries, and context window
//! management.

use std::collections::HashMap;

/// Aggregated usage statistics for a single model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelUsage {
    /// Total input (prompt) tokens consumed.
    pub input_tokens: u64,
    /// Total output (completion) tokens consumed.
    pub output_tokens: u64,
    /// Number of API requests made.
    pub request_count: u32,
}

impl ModelUsage {
    /// Total tokens (input + output) for this model.
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// Session-level token usage tracker across all models.
#[derive(Debug, Clone, Default)]
pub struct UsageTracker {
    per_model: HashMap<String, ModelUsage>,
}

impl UsageTracker {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a completed API request's token usage.
    pub fn record_usage(&mut self, model: &str, input_tokens: u64, output_tokens: u64) {
        let entry = self.per_model.entry(model.to_string()).or_default();
        entry.input_tokens += input_tokens;
        entry.output_tokens += output_tokens;
        entry.request_count += 1;
    }

    /// Total tokens (input + output) across all models.
    pub fn total_tokens(&self) -> u64 {
        self.per_model
            .values()
            .map(|u| u.input_tokens + u.output_tokens)
            .sum()
    }

    /// Total input tokens across all models.
    pub fn total_input_tokens(&self) -> u64 {
        self.per_model.values().map(|u| u.input_tokens).sum()
    }

    /// Total output tokens across all models.
    pub fn total_output_tokens(&self) -> u64 {
        self.per_model.values().map(|u| u.output_tokens).sum()
    }

    /// Total number of API requests across all models.
    pub fn total_requests(&self) -> u32 {
        self.per_model.values().map(|u| u.request_count).sum()
    }

    /// Per-model usage breakdown.
    pub fn per_model_summary(&self) -> HashMap<String, ModelUsage> {
        self.per_model.clone()
    }

    /// Usage for a specific model.
    pub fn model_usage(&self, model: &str) -> Option<&ModelUsage> {
        self.per_model.get(model)
    }

    /// Reset all tracked usage.
    pub fn reset(&mut self) {
        self.per_model.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tracker() {
        let t = UsageTracker::new();
        assert_eq!(t.total_tokens(), 0);
        assert_eq!(t.total_requests(), 0);
        assert!(t.per_model_summary().is_empty());
    }

    #[test]
    fn record_single_model() {
        let mut t = UsageTracker::new();
        t.record_usage("claude-3-sonnet", 100, 50);
        t.record_usage("claude-3-sonnet", 200, 100);
        assert_eq!(t.total_tokens(), 450);
        assert_eq!(t.total_input_tokens(), 300);
        assert_eq!(t.total_output_tokens(), 150);
        assert_eq!(t.total_requests(), 2);

        let usage = t.model_usage("claude-3-sonnet").unwrap();
        assert_eq!(usage.request_count, 2);
    }

    #[test]
    fn record_multiple_models() {
        let mut t = UsageTracker::new();
        t.record_usage("sonnet", 100, 50);
        t.record_usage("haiku", 50, 25);
        assert_eq!(t.total_tokens(), 225);
        assert_eq!(t.per_model_summary().len(), 2);
    }

    #[test]
    fn reset_clears_everything() {
        let mut t = UsageTracker::new();
        t.record_usage("sonnet", 100, 50);
        t.reset();
        assert_eq!(t.total_tokens(), 0);
        assert!(t.per_model_summary().is_empty());
    }

    #[test]
    fn model_usage_total() {
        let u = ModelUsage {
            input_tokens: 100,
            output_tokens: 50,
            request_count: 1,
        };
        assert_eq!(u.total_tokens(), 150);
    }
}
