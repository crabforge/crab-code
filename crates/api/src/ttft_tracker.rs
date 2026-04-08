//! Time-to-first-token (TTFT) latency tracking.
//!
//! Measures the elapsed time from sending an LLM request to receiving the
//! first streamed token. Maintains a rolling history for computing averages
//! and detecting latency regressions.

use std::time::Instant;

/// Tracks time-to-first-token latency across multiple requests.
///
/// Usage:
/// 1. Call `record_request_start()` when sending a request.
/// 2. Call `record_first_token()` when the first streaming event arrives.
/// 3. Query `average_ttft_ms()` or `last_ttft_ms()` for metrics.
#[derive(Debug)]
pub struct TtftTracker {
    /// Start time of the current in-flight request, if any.
    current_start: Option<Instant>,
    /// Recorded TTFT durations in milliseconds (most recent last).
    history: Vec<f64>,
}

impl TtftTracker {
    /// Create a new tracker with no history.
    pub fn new() -> Self {
        Self {
            current_start: None,
            history: Vec::new(),
        }
    }

    /// Mark the start of a new LLM request.
    ///
    /// If a previous request was started but never completed, it is silently
    /// discarded.
    pub fn record_request_start(&mut self) {
        self.current_start = Some(Instant::now());
    }

    /// Mark the arrival of the first token for the current request.
    ///
    /// Records the elapsed time since `record_request_start()`. No-op if
    /// no request is currently in flight.
    pub fn record_first_token(&mut self) {
        if let Some(start) = self.current_start.take() {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            self.history.push(elapsed_ms);
        }
    }

    /// Average TTFT in milliseconds across all recorded requests.
    ///
    /// Returns `None` if no requests have been recorded.
    pub fn average_ttft_ms(&self) -> Option<f64> {
        if self.history.is_empty() {
            return None;
        }
        let sum: f64 = self.history.iter().sum();
        Some(sum / self.history.len() as f64)
    }

    /// TTFT of the most recent request in milliseconds.
    pub fn last_ttft_ms(&self) -> Option<f64> {
        self.history.last().copied()
    }

    /// Number of recorded TTFT measurements.
    pub fn count(&self) -> usize {
        self.history.len()
    }

    /// Reset all history.
    pub fn reset(&mut self) {
        self.current_start = None;
        self.history.clear();
    }
}

impl Default for TtftTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_has_no_history() {
        let t = TtftTracker::new();
        assert_eq!(t.count(), 0);
        assert!(t.average_ttft_ms().is_none());
        assert!(t.last_ttft_ms().is_none());
    }

    #[test]
    fn record_first_token_without_start_is_noop() {
        let mut t = TtftTracker::new();
        t.record_first_token();
        assert_eq!(t.count(), 0);
    }

    #[test]
    fn record_produces_measurement() {
        let mut t = TtftTracker::new();
        t.record_request_start();
        // Immediately record first token — duration should be tiny but non-negative.
        t.record_first_token();
        assert_eq!(t.count(), 1);
        assert!(t.last_ttft_ms().unwrap() >= 0.0);
    }

    #[test]
    fn reset_clears_everything() {
        let mut t = TtftTracker::new();
        t.record_request_start();
        t.record_first_token();
        assert_eq!(t.count(), 1);
        t.reset();
        assert_eq!(t.count(), 0);
        assert!(t.average_ttft_ms().is_none());
    }
}
