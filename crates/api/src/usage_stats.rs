//! Usage statistics and cost estimation.
//!
//! `UsageStats` tracks token usage aggregated by model and provider.
//! `cost_estimate` computes dollar cost from known pricing tables.
//! `SessionUsageReport` provides a human-readable summary.

use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Pricing table
// ---------------------------------------------------------------------------

/// Per-token pricing for a model (in USD per token).
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_field_names)]
struct ModelPricing {
    input_per_token: f64,
    output_per_token: f64,
    cache_read_per_token: f64,
}

/// Get known pricing for a model ID. Returns None for unknown models.
fn known_pricing(model_id: &str) -> Option<ModelPricing> {
    // Prices in USD per token (as of early 2025 Anthropic/OpenAI pricing).
    let pricing = match model_id {
        // Anthropic Claude models
        s if s.contains("opus") => ModelPricing {
            input_per_token: 15.0 / 1_000_000.0,
            output_per_token: 75.0 / 1_000_000.0,
            cache_read_per_token: 1.5 / 1_000_000.0,
        },
        s if s.contains("sonnet") => ModelPricing {
            input_per_token: 3.0 / 1_000_000.0,
            output_per_token: 15.0 / 1_000_000.0,
            cache_read_per_token: 0.3 / 1_000_000.0,
        },
        s if s.contains("haiku") => ModelPricing {
            input_per_token: 0.25 / 1_000_000.0,
            output_per_token: 1.25 / 1_000_000.0,
            cache_read_per_token: 0.03 / 1_000_000.0,
        },
        // OpenAI models
        s if s.starts_with("gpt-4o") => ModelPricing {
            input_per_token: 2.5 / 1_000_000.0,
            output_per_token: 10.0 / 1_000_000.0,
            cache_read_per_token: 1.25 / 1_000_000.0,
        },
        s if s.starts_with("gpt-4-turbo") => ModelPricing {
            input_per_token: 10.0 / 1_000_000.0,
            output_per_token: 30.0 / 1_000_000.0,
            cache_read_per_token: 5.0 / 1_000_000.0,
        },
        s if s.starts_with("gpt-3.5") => ModelPricing {
            input_per_token: 0.5 / 1_000_000.0,
            output_per_token: 1.5 / 1_000_000.0,
            cache_read_per_token: 0.25 / 1_000_000.0,
        },
        _ => return None,
    };
    Some(pricing)
}

/// Estimate cost in USD for given token counts.
///
/// Returns `None` if the model is not in the known pricing table.
#[must_use]
pub fn cost_estimate(model_id: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    let pricing = known_pricing(model_id)?;
    Some(
        input_tokens as f64 * pricing.input_per_token
            + output_tokens as f64 * pricing.output_per_token,
    )
}

/// Estimate cost including cache read tokens.
#[must_use]
pub fn cost_estimate_with_cache(
    model_id: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
) -> Option<f64> {
    let pricing = known_pricing(model_id)?;
    Some(
        input_tokens as f64 * pricing.input_per_token
            + output_tokens as f64 * pricing.output_per_token
            + cache_read_tokens as f64 * pricing.cache_read_per_token,
    )
}

// ---------------------------------------------------------------------------
// ModelUsage — per-model aggregated stats
// ---------------------------------------------------------------------------

/// Aggregated usage for a single model.
#[derive(Debug, Clone, Default)]
pub struct ModelUsage {
    pub model_id: String,
    pub request_count: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub estimated_cost: Option<f64>,
}

impl ModelUsage {
    fn new(model_id: &str) -> Self {
        Self {
            model_id: model_id.to_string(),
            ..Default::default()
        }
    }

    /// Total tokens (input + output).
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

// ---------------------------------------------------------------------------
// UsageStats
// ---------------------------------------------------------------------------

/// Tracks token usage aggregated by model.
#[derive(Debug, Clone, Default)]
pub struct UsageStats {
    by_model: HashMap<String, ModelUsage>,
}

impl UsageStats {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record usage for a request.
    pub fn record(
        &mut self,
        model_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
    ) {
        let entry = self
            .by_model
            .entry(model_id.to_string())
            .or_insert_with(|| ModelUsage::new(model_id));
        entry.request_count += 1;
        entry.input_tokens += input_tokens;
        entry.output_tokens += output_tokens;
        entry.cache_read_tokens += cache_read_tokens;
        entry.cache_creation_tokens += cache_creation_tokens;
        entry.estimated_cost = cost_estimate_with_cache(
            model_id,
            entry.input_tokens,
            entry.output_tokens,
            entry.cache_read_tokens,
        );
    }

    /// Get usage for a specific model.
    #[must_use]
    pub fn model_usage(&self, model_id: &str) -> Option<&ModelUsage> {
        self.by_model.get(model_id)
    }

    /// All model usages.
    #[must_use]
    pub fn all_models(&self) -> Vec<&ModelUsage> {
        self.by_model.values().collect()
    }

    /// Total input tokens across all models.
    #[must_use]
    pub fn total_input_tokens(&self) -> u64 {
        self.by_model.values().map(|m| m.input_tokens).sum()
    }

    /// Total output tokens across all models.
    #[must_use]
    pub fn total_output_tokens(&self) -> u64 {
        self.by_model.values().map(|m| m.output_tokens).sum()
    }

    /// Total estimated cost across all models (None if any model has unknown pricing).
    #[must_use]
    pub fn total_estimated_cost(&self) -> Option<f64> {
        let mut total = 0.0;
        for usage in self.by_model.values() {
            total += usage.estimated_cost?;
        }
        Some(total)
    }

    /// Total request count.
    #[must_use]
    pub fn total_requests(&self) -> u32 {
        self.by_model.values().map(|m| m.request_count).sum()
    }

    /// Generate a session usage report.
    #[must_use]
    pub fn report(&self) -> SessionUsageReport {
        let mut by_model: Vec<ModelUsage> = self.by_model.values().cloned().collect();
        by_model.sort_by(|a, b| b.total_tokens().cmp(&a.total_tokens()));

        SessionUsageReport {
            total_input: self.total_input_tokens(),
            total_output: self.total_output_tokens(),
            total_cost: self.total_estimated_cost(),
            total_requests: self.total_requests(),
            by_model,
        }
    }
}

// ---------------------------------------------------------------------------
// SessionUsageReport
// ---------------------------------------------------------------------------

/// Human-readable session usage report.
#[derive(Debug, Clone)]
pub struct SessionUsageReport {
    pub total_input: u64,
    pub total_output: u64,
    pub total_cost: Option<f64>,
    pub total_requests: u32,
    pub by_model: Vec<ModelUsage>,
}

impl fmt::Display for SessionUsageReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Session Usage Report")?;
        writeln!(f, "===================")?;
        writeln!(f, "Total requests:      {}", self.total_requests)?;
        writeln!(f, "Total input tokens:  {}", self.total_input)?;
        writeln!(f, "Total output tokens: {}", self.total_output)?;
        if let Some(cost) = self.total_cost {
            writeln!(f, "Estimated cost:      ${cost:.4}")?;
        } else {
            writeln!(f, "Estimated cost:      (unknown pricing)")?;
        }
        writeln!(f)?;

        if !self.by_model.is_empty() {
            writeln!(f, "By Model:")?;
            writeln!(
                f,
                "{:<35} {:>6} {:>10} {:>10} {:>10}",
                "Model", "Reqs", "Input", "Output", "Cost"
            )?;
            writeln!(f, "{}", "-".repeat(75))?;
            for model in &self.by_model {
                let cost_str = model
                    .estimated_cost
                    .map_or_else(|| "N/A".to_string(), |c| format!("${c:.4}"));
                writeln!(
                    f,
                    "{:<35} {:>6} {:>10} {:>10} {:>10}",
                    model.model_id,
                    model.request_count,
                    model.input_tokens,
                    model.output_tokens,
                    cost_str
                )?;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- cost_estimate --

    #[test]
    fn cost_estimate_sonnet() {
        let cost = cost_estimate("claude-sonnet-4-20250514", 1_000_000, 1_000_000);
        assert!(cost.is_some());
        let c = cost.unwrap();
        // sonnet: $3/M input + $15/M output = $18
        assert!((c - 18.0).abs() < 0.01);
    }

    #[test]
    fn cost_estimate_opus() {
        let cost = cost_estimate("claude-opus-4-20250514", 1_000_000, 1_000_000);
        let c = cost.unwrap();
        // opus: $15/M input + $75/M output = $90
        assert!((c - 90.0).abs() < 0.01);
    }

    #[test]
    fn cost_estimate_haiku() {
        let cost = cost_estimate("claude-haiku-3-5", 1_000_000, 1_000_000);
        let c = cost.unwrap();
        // haiku: $0.25/M + $1.25/M = $1.50
        assert!((c - 1.50).abs() < 0.01);
    }

    #[test]
    fn cost_estimate_unknown_model() {
        assert!(cost_estimate("local-llama", 1000, 1000).is_none());
    }

    #[test]
    fn cost_estimate_with_cache_tokens() {
        let cost = cost_estimate_with_cache("claude-sonnet-4-20250514", 500_000, 100_000, 400_000);
        assert!(cost.is_some());
        let c = cost.unwrap();
        // 500k * $3/M + 100k * $15/M + 400k * $0.3/M = $1.5 + $1.5 + $0.12 = $3.12
        assert!((c - 3.12).abs() < 0.01);
    }

    // -- UsageStats --

    #[test]
    fn stats_empty() {
        let stats = UsageStats::new();
        assert_eq!(stats.total_input_tokens(), 0);
        assert_eq!(stats.total_output_tokens(), 0);
        assert_eq!(stats.total_requests(), 0);
    }

    #[test]
    fn stats_record_single_model() {
        let mut stats = UsageStats::new();
        stats.record("claude-sonnet-4-20250514", 1000, 500, 200, 0);
        stats.record("claude-sonnet-4-20250514", 800, 300, 100, 0);

        assert_eq!(stats.total_requests(), 2);
        assert_eq!(stats.total_input_tokens(), 1800);
        assert_eq!(stats.total_output_tokens(), 800);

        let model = stats.model_usage("claude-sonnet-4-20250514").unwrap();
        assert_eq!(model.request_count, 2);
        assert_eq!(model.input_tokens, 1800);
        assert!(model.estimated_cost.is_some());
    }

    #[test]
    fn stats_record_multiple_models() {
        let mut stats = UsageStats::new();
        stats.record("claude-sonnet-4-20250514", 1000, 500, 0, 0);
        stats.record("claude-haiku-3-5", 2000, 1000, 0, 0);

        assert_eq!(stats.total_requests(), 2);
        assert_eq!(stats.all_models().len(), 2);
        assert!(stats.total_estimated_cost().is_some());
    }

    #[test]
    fn stats_unknown_model_no_total_cost() {
        let mut stats = UsageStats::new();
        stats.record("claude-sonnet-4-20250514", 1000, 500, 0, 0);
        stats.record("local-llama", 2000, 1000, 0, 0);

        // Total cost is None because local-llama has no pricing.
        assert!(stats.total_estimated_cost().is_none());
    }

    // -- SessionUsageReport --

    #[test]
    fn report_format() {
        let mut stats = UsageStats::new();
        stats.record("claude-sonnet-4-20250514", 10000, 5000, 0, 0);
        let report = stats.report();

        assert_eq!(report.total_input, 10000);
        assert_eq!(report.total_output, 5000);
        assert_eq!(report.total_requests, 1);
        assert_eq!(report.by_model.len(), 1);

        let display = format!("{report}");
        assert!(display.contains("Session Usage Report"));
        assert!(display.contains("10000"));
        assert!(display.contains("5000"));
    }

    #[test]
    fn report_sorted_by_total_tokens() {
        let mut stats = UsageStats::new();
        stats.record("claude-haiku-3-5", 100, 50, 0, 0);
        stats.record("claude-sonnet-4-20250514", 10000, 5000, 0, 0);
        let report = stats.report();

        // Sonnet (15000 total) should come before haiku (150 total).
        assert_eq!(report.by_model[0].model_id, "claude-sonnet-4-20250514");
        assert_eq!(report.by_model[1].model_id, "claude-haiku-3-5");
    }

    // -- ModelUsage --

    #[test]
    fn model_usage_total() {
        let mut m = ModelUsage::new("test");
        m.input_tokens = 1000;
        m.output_tokens = 500;
        assert_eq!(m.total_tokens(), 1500);
    }
}
