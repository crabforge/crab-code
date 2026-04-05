//! Tool usage analytics: recording, aggregation, efficiency scoring, and
//! session-level usage summaries.

use std::collections::HashMap;

// ── Data model ─────────────────────────────────────────────────────────

/// A single record of a tool invocation.
#[derive(Debug, Clone)]
pub struct ToolUsageRecord {
    pub tool_name: String,
    /// Monotonic timestamp in milliseconds (relative to session start).
    pub timestamp_ms: u64,
    /// Wall-clock duration of the tool execution.
    pub duration_ms: u64,
    pub success: bool,
    /// Approximate size of the JSON input in bytes.
    pub input_size: usize,
    /// Approximate size of the output in bytes.
    pub output_size: usize,
}

/// Aggregated statistics for a single tool.
#[derive(Debug, Clone)]
pub struct ToolStats {
    pub tool_name: String,
    pub call_count: u64,
    pub success_count: u64,
    pub total_duration_ms: u64,
    pub total_input_bytes: u64,
    pub total_output_bytes: u64,
}

impl ToolStats {
    fn new(name: &str) -> Self {
        Self {
            tool_name: name.to_string(),
            call_count: 0,
            success_count: 0,
            total_duration_ms: 0,
            total_input_bytes: 0,
            total_output_bytes: 0,
        }
    }

    /// Success rate in the range `[0.0, 1.0]`.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.call_count == 0 {
            return 0.0;
        }
        self.success_count as f64 / self.call_count as f64
    }

    /// Average duration in milliseconds.
    #[must_use]
    pub fn avg_duration_ms(&self) -> f64 {
        if self.call_count == 0 {
            return 0.0;
        }
        self.total_duration_ms as f64 / self.call_count as f64
    }
}

/// Session-level tool usage summary.
#[derive(Debug, Clone)]
pub struct ToolUsageSummary {
    pub total_calls: u64,
    pub total_successes: u64,
    pub total_failures: u64,
    pub total_duration_ms: u64,
    /// Tools sorted by call count descending.
    pub top_tools: Vec<ToolStats>,
    /// Session wall-clock span (max timestamp - min timestamp) in ms.
    pub session_span_ms: u64,
}

impl std::fmt::Display for ToolUsageSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Tool Usage Summary")?;
        writeln!(
            f,
            "  Total calls: {} ({} ok, {} failed)",
            self.total_calls, self.total_successes, self.total_failures
        )?;
        writeln!(f, "  Total duration: {} ms", self.total_duration_ms)?;
        if !self.top_tools.is_empty() {
            writeln!(f, "  Top tools:")?;
            for s in &self.top_tools {
                writeln!(
                    f,
                    "    {} — {} calls, {:.0}% success, {:.0} ms avg",
                    s.tool_name,
                    s.call_count,
                    s.success_rate() * 100.0,
                    s.avg_duration_ms()
                )?;
            }
        }
        Ok(())
    }
}

// ── Analytics engine ───────────────────────────────────────────────────

/// Collects `ToolUsageRecord`s and produces aggregated analytics.
#[derive(Debug, Clone, Default)]
pub struct ToolAnalytics {
    records: Vec<ToolUsageRecord>,
}

impl ToolAnalytics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// Record a tool invocation.
    pub fn record(&mut self, rec: ToolUsageRecord) {
        self.records.push(rec);
    }

    /// Number of records stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether there are no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Read-only access to all records.
    #[must_use]
    pub fn records(&self) -> &[ToolUsageRecord] {
        &self.records
    }

    /// Compute per-tool aggregated statistics.
    #[must_use]
    pub fn aggregate(&self) -> HashMap<String, ToolStats> {
        let mut map: HashMap<String, ToolStats> = HashMap::new();
        for r in &self.records {
            let stats = map
                .entry(r.tool_name.clone())
                .or_insert_with(|| ToolStats::new(&r.tool_name));
            stats.call_count += 1;
            if r.success {
                stats.success_count += 1;
            }
            stats.total_duration_ms += r.duration_ms;
            stats.total_input_bytes += r.input_size as u64;
            stats.total_output_bytes += r.output_size as u64;
        }
        map
    }

    /// Return the top-N most called tools.
    #[must_use]
    pub fn top_tools(&self, n: usize) -> Vec<ToolStats> {
        let agg = self.aggregate();
        let mut sorted: Vec<ToolStats> = agg.into_values().collect();
        sorted.sort_by(|a, b| b.call_count.cmp(&a.call_count));
        sorted.truncate(n);
        sorted
    }

    /// Efficiency score for a tool: `success_rate * speed_factor`.
    ///
    /// `speed_factor` = `1.0 / (1.0 + avg_duration_ms / 1000.0)` — faster
    /// tools score higher. Returns 0.0 if the tool has no records.
    #[must_use]
    pub fn tool_efficiency_score(&self, tool_name: &str) -> f64 {
        let agg = self.aggregate();
        let Some(stats) = agg.get(tool_name) else {
            return 0.0;
        };
        let speed_factor = 1.0 / (1.0 + stats.avg_duration_ms() / 1000.0);
        stats.success_rate() * speed_factor
    }

    /// Produce a session-level summary.
    #[must_use]
    pub fn session_tool_summary(&self) -> ToolUsageSummary {
        let agg = self.aggregate();
        let total_calls: u64 = agg.values().map(|s| s.call_count).sum();
        let total_successes: u64 = agg.values().map(|s| s.success_count).sum();
        let total_duration_ms: u64 = agg.values().map(|s| s.total_duration_ms).sum();

        let session_span_ms = if self.records.len() < 2 {
            0
        } else {
            let min_ts = self.records.iter().map(|r| r.timestamp_ms).min().unwrap_or(0);
            let max_ts = self.records.iter().map(|r| r.timestamp_ms).max().unwrap_or(0);
            max_ts.saturating_sub(min_ts)
        };

        let mut top: Vec<ToolStats> = agg.into_values().collect();
        top.sort_by(|a, b| b.call_count.cmp(&a.call_count));

        ToolUsageSummary {
            total_calls,
            total_successes,
            total_failures: total_calls - total_successes,
            total_duration_ms,
            top_tools: top,
            session_span_ms,
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(name: &str, ts: u64, dur: u64, success: bool) -> ToolUsageRecord {
        ToolUsageRecord {
            tool_name: name.to_string(),
            timestamp_ms: ts,
            duration_ms: dur,
            success,
            input_size: 100,
            output_size: 200,
        }
    }

    #[test]
    fn empty_analytics() {
        let a = ToolAnalytics::new();
        assert!(a.is_empty());
        assert_eq!(a.len(), 0);
        assert!(a.aggregate().is_empty());
        let summary = a.session_tool_summary();
        assert_eq!(summary.total_calls, 0);
    }

    #[test]
    fn record_and_len() {
        let mut a = ToolAnalytics::new();
        a.record(make_record("read", 0, 10, true));
        a.record(make_record("write", 100, 20, true));
        assert_eq!(a.len(), 2);
        assert!(!a.is_empty());
    }

    #[test]
    fn aggregate_counts() {
        let mut a = ToolAnalytics::new();
        a.record(make_record("read", 0, 10, true));
        a.record(make_record("read", 100, 20, false));
        a.record(make_record("write", 200, 30, true));
        let agg = a.aggregate();
        assert_eq!(agg["read"].call_count, 2);
        assert_eq!(agg["read"].success_count, 1);
        assert_eq!(agg["write"].call_count, 1);
    }

    #[test]
    fn success_rate() {
        let mut stats = ToolStats::new("t");
        assert_eq!(stats.success_rate(), 0.0);
        stats.call_count = 4;
        stats.success_count = 3;
        assert!((stats.success_rate() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn avg_duration() {
        let mut stats = ToolStats::new("t");
        assert_eq!(stats.avg_duration_ms(), 0.0);
        stats.call_count = 2;
        stats.total_duration_ms = 100;
        assert!((stats.avg_duration_ms() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn top_tools_ordering() {
        let mut a = ToolAnalytics::new();
        for i in 0..5 {
            a.record(make_record("read", i * 10, 10, true));
        }
        for i in 0..3 {
            a.record(make_record("write", 100 + i * 10, 10, true));
        }
        a.record(make_record("bash", 200, 10, true));
        let top = a.top_tools(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].tool_name, "read");
        assert_eq!(top[1].tool_name, "write");
    }

    #[test]
    fn top_tools_truncates() {
        let mut a = ToolAnalytics::new();
        a.record(make_record("read", 0, 10, true));
        let top = a.top_tools(5);
        assert_eq!(top.len(), 1);
    }

    #[test]
    fn efficiency_score_unknown_tool() {
        let a = ToolAnalytics::new();
        assert_eq!(a.tool_efficiency_score("nonexistent"), 0.0);
    }

    #[test]
    fn efficiency_score_fast_successful() {
        let mut a = ToolAnalytics::new();
        // 100% success, 10ms avg → speed_factor = 1/(1+0.01) ≈ 0.99
        a.record(make_record("read", 0, 10, true));
        a.record(make_record("read", 10, 10, true));
        let score = a.tool_efficiency_score("read");
        assert!(score > 0.98);
        assert!(score <= 1.0);
    }

    #[test]
    fn efficiency_score_slow_tool() {
        let mut a = ToolAnalytics::new();
        // 100% success, 5000ms avg → speed_factor = 1/(1+5) = 0.167
        a.record(make_record("bash", 0, 5000, true));
        let score = a.tool_efficiency_score("bash");
        assert!(score > 0.15 && score < 0.20);
    }

    #[test]
    fn efficiency_score_mixed() {
        let mut a = ToolAnalytics::new();
        // 50% success, 100ms avg → sr=0.5, sf=1/(1+0.1)=0.909 → ~0.455
        a.record(make_record("edit", 0, 100, true));
        a.record(make_record("edit", 100, 100, false));
        let score = a.tool_efficiency_score("edit");
        assert!(score > 0.40 && score < 0.50);
    }

    #[test]
    fn session_summary_totals() {
        let mut a = ToolAnalytics::new();
        a.record(make_record("read", 0, 10, true));
        a.record(make_record("read", 50, 20, false));
        a.record(make_record("write", 100, 30, true));
        let s = a.session_tool_summary();
        assert_eq!(s.total_calls, 3);
        assert_eq!(s.total_successes, 2);
        assert_eq!(s.total_failures, 1);
        assert_eq!(s.total_duration_ms, 60);
    }

    #[test]
    fn session_span_single_record() {
        let mut a = ToolAnalytics::new();
        a.record(make_record("read", 500, 10, true));
        let s = a.session_tool_summary();
        assert_eq!(s.session_span_ms, 0);
    }

    #[test]
    fn session_span_multiple_records() {
        let mut a = ToolAnalytics::new();
        a.record(make_record("read", 100, 10, true));
        a.record(make_record("write", 500, 10, true));
        a.record(make_record("bash", 300, 10, true));
        let s = a.session_tool_summary();
        assert_eq!(s.session_span_ms, 400); // 500 - 100
    }

    #[test]
    fn summary_display() {
        let mut a = ToolAnalytics::new();
        a.record(make_record("read", 0, 10, true));
        a.record(make_record("write", 100, 20, true));
        let s = a.session_tool_summary();
        let text = s.to_string();
        assert!(text.contains("Tool Usage Summary"));
        assert!(text.contains("Total calls: 2"));
    }

    #[test]
    fn aggregate_bytes() {
        let mut a = ToolAnalytics::new();
        a.record(ToolUsageRecord {
            tool_name: "read".into(),
            timestamp_ms: 0,
            duration_ms: 10,
            success: true,
            input_size: 50,
            output_size: 300,
        });
        a.record(ToolUsageRecord {
            tool_name: "read".into(),
            timestamp_ms: 10,
            duration_ms: 10,
            success: true,
            input_size: 150,
            output_size: 700,
        });
        let agg = a.aggregate();
        assert_eq!(agg["read"].total_input_bytes, 200);
        assert_eq!(agg["read"].total_output_bytes, 1000);
    }

    #[test]
    fn records_accessor() {
        let mut a = ToolAnalytics::new();
        a.record(make_record("read", 0, 10, true));
        assert_eq!(a.records().len(), 1);
        assert_eq!(a.records()[0].tool_name, "read");
    }

    #[test]
    fn default_trait() {
        let a = ToolAnalytics::default();
        assert!(a.is_empty());
    }
}
