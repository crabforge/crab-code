use std::fmt;

use crab_core::model::{CostTracker, TokenUsage};

/// Session-level cost accumulator — thin wrapper around core `CostTracker`
/// with session-specific helpers and formatted output.
#[derive(Default)]
pub struct CostAccumulator {
    pub tracker: CostTracker,
    /// Number of API calls recorded.
    pub api_calls: u64,
}

impl CostAccumulator {
    /// Record a single API response's usage and cost.
    pub fn record(&mut self, usage: &TokenUsage, cost: f64) {
        self.tracker.record(usage, cost);
        self.api_calls += 1;
    }

    pub fn total_tokens(&self) -> u64 {
        self.tracker.total_tokens()
    }

    pub fn total_cost_usd(&self) -> f64 {
        self.tracker.total_cost_usd
    }

    /// Total cache-related tokens (read + creation).
    pub fn total_cache_tokens(&self) -> u64 {
        self.tracker.total_cache_read_tokens + self.tracker.total_cache_creation_tokens
    }

    /// Format as a compact summary line for TUI display.
    pub fn summary_line(&self) -> String {
        format!(
            "tokens: {}in/{}out | cache: {}r/{}w | cost: ${:.4} | calls: {}",
            self.tracker.total_input_tokens,
            self.tracker.total_output_tokens,
            self.tracker.total_cache_read_tokens,
            self.tracker.total_cache_creation_tokens,
            self.tracker.total_cost_usd,
            self.api_calls,
        )
    }
}

impl fmt::Display for CostAccumulator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary_line())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero() {
        let acc = CostAccumulator::default();
        assert_eq!(acc.total_tokens(), 0);
        assert_eq!(acc.total_cost_usd(), 0.0);
        assert_eq!(acc.api_calls, 0);
        assert_eq!(acc.total_cache_tokens(), 0);
    }

    #[test]
    fn record_accumulates() {
        let mut acc = CostAccumulator::default();
        acc.record(
            &TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 20,
                cache_creation_tokens: 10,
            },
            0.01,
        );
        acc.record(
            &TokenUsage {
                input_tokens: 200,
                output_tokens: 100,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            0.02,
        );
        assert_eq!(acc.total_tokens(), 450);
        assert_eq!(acc.api_calls, 2);
        assert_eq!(acc.total_cache_tokens(), 30);
        assert!((acc.total_cost_usd() - 0.03).abs() < f64::EPSILON);
    }

    #[test]
    fn summary_line_format() {
        let mut acc = CostAccumulator::default();
        acc.record(
            &TokenUsage {
                input_tokens: 1000,
                output_tokens: 500,
                cache_read_tokens: 200,
                cache_creation_tokens: 100,
            },
            0.015,
        );
        let line = acc.summary_line();
        assert!(line.contains("1000in"));
        assert!(line.contains("500out"));
        assert!(line.contains("200r"));
        assert!(line.contains("100w"));
        assert!(line.contains("$0.0150"));
        assert!(line.contains("calls: 1"));
    }

    #[test]
    fn display_trait() {
        let acc = CostAccumulator::default();
        let s = format!("{acc}");
        assert!(s.contains("tokens:"));
    }
}
