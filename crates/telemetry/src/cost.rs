//! Per-provider token cost tracking.
//!
//! All cost data stays local — never sent to any external service.
//! Costs are stored in `~/.crab/telemetry/costs.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Pricing configuration ─────────────────────────────────────────────

/// Price per million tokens for a specific model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Cost per 1M input tokens (USD).
    pub input_per_million: f64,
    /// Cost per 1M output tokens (USD).
    pub output_per_million: f64,
}

impl ModelPricing {
    #[must_use]
    pub fn new(input_per_million: f64, output_per_million: f64) -> Self {
        Self {
            input_per_million,
            output_per_million,
        }
    }

    /// Calculate cost for a given number of input and output tokens.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn calculate(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        (input_tokens as f64).mul_add(
            self.input_per_million,
            output_tokens as f64 * self.output_per_million,
        ) / 1_000_000.0
    }
}

/// Configurable pricing table mapping `"provider/model"` to pricing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PricingTable {
    /// Map of `"provider/model"` → pricing. Example key: `"anthropic/claude-sonnet-4-20250514"`.
    #[serde(default)]
    pub models: HashMap<String, ModelPricing>,
}

impl PricingTable {
    /// Create a pricing table with sensible defaults for common models.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut models = HashMap::new();
        // Anthropic (approximate public pricing)
        models.insert(
            "anthropic/claude-sonnet-4-20250514".to_string(),
            ModelPricing::new(3.0, 15.0),
        );
        models.insert(
            "anthropic/claude-haiku-3-5".to_string(),
            ModelPricing::new(0.25, 1.25),
        );
        // OpenAI (approximate)
        models.insert("openai/gpt-4o".to_string(), ModelPricing::new(2.5, 10.0));
        models.insert(
            "openai/gpt-4o-mini".to_string(),
            ModelPricing::new(0.15, 0.6),
        );
        Self { models }
    }

    /// Look up pricing for a provider/model combination.
    #[must_use]
    pub fn get(&self, provider: &str, model: &str) -> Option<&ModelPricing> {
        let key = format!("{provider}/{model}");
        self.models.get(&key)
    }

    /// Set pricing for a provider/model combination.
    pub fn set(&mut self, provider: &str, model: &str, pricing: ModelPricing) {
        let key = format!("{provider}/{model}");
        self.models.insert(key, pricing);
    }
}

// ── Usage record ──────────────────────────────────────────────────────

/// A single API call usage record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Provider name.
    pub provider: String,
    /// Model name.
    pub model: String,
    /// Number of input tokens.
    pub input_tokens: u64,
    /// Number of output tokens.
    pub output_tokens: u64,
    /// Computed cost in USD (0.0 if pricing unknown).
    pub cost_usd: f64,
    /// Optional session ID for grouping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

// ── Cost tracker ──────────────────────────────────────────────────────

/// Accumulates token usage and cost data. All data stays local.
pub struct CostTracker {
    pricing: PricingTable,
    records: Vec<UsageRecord>,
    store_path: PathBuf,
}

impl std::fmt::Debug for CostTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CostTracker")
            .field("records_count", &self.records.len())
            .field("store_path", &self.store_path)
            .finish_non_exhaustive()
    }
}

impl CostTracker {
    /// Create a new tracker with the given pricing table and storage path.
    #[must_use]
    pub fn new(pricing: PricingTable, store_path: PathBuf) -> Self {
        Self {
            pricing,
            records: Vec::new(),
            store_path,
        }
    }

    /// Default cost store path: `~/.crab/telemetry/costs.json`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        crab_common::path::home_dir()
            .join(".crab")
            .join("telemetry")
            .join("costs.json")
    }

    /// Record a single API call's token usage.
    pub fn record(
        &mut self,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        session_id: Option<String>,
    ) {
        let cost_usd = self
            .pricing
            .get(provider, model)
            .map_or(0.0, |p| p.calculate(input_tokens, output_tokens));

        self.records.push(UsageRecord {
            timestamp: now_iso8601(),
            provider: provider.to_string(),
            model: model.to_string(),
            input_tokens,
            output_tokens,
            cost_usd,
            session_id,
        });
    }

    /// Get all usage records.
    #[must_use]
    pub fn records(&self) -> &[UsageRecord] {
        &self.records
    }

    /// Total cost across all records.
    #[must_use]
    pub fn total_cost(&self) -> f64 {
        self.records.iter().map(|r| r.cost_usd).sum()
    }

    /// Total cost for a specific provider.
    #[must_use]
    pub fn cost_by_provider(&self, provider: &str) -> f64 {
        self.records
            .iter()
            .filter(|r| r.provider == provider)
            .map(|r| r.cost_usd)
            .sum()
    }

    /// Total tokens (input + output) across all records.
    #[must_use]
    pub fn total_tokens(&self) -> (u64, u64) {
        self.records.iter().fold((0, 0), |(i, o), r| {
            (i + r.input_tokens, o + r.output_tokens)
        })
    }

    /// Summary of costs grouped by provider.
    #[must_use]
    pub fn summary_by_provider(&self) -> HashMap<String, ProviderSummary> {
        let mut map: HashMap<String, ProviderSummary> = HashMap::new();
        for r in &self.records {
            let entry = map.entry(r.provider.clone()).or_default();
            entry.input_tokens += r.input_tokens;
            entry.output_tokens += r.output_tokens;
            entry.cost_usd += r.cost_usd;
            entry.request_count += 1;
        }
        map
    }

    /// Get the pricing table (for inspection/modification).
    #[must_use]
    pub fn pricing(&self) -> &PricingTable {
        &self.pricing
    }

    /// Get a mutable reference to the pricing table.
    pub fn pricing_mut(&mut self) -> &mut PricingTable {
        &mut self.pricing
    }

    // ── Persistence ───────────────────────────────────────────────────

    /// Load records from disk, appending to any in-memory records.
    pub fn load(&mut self) -> crab_common::Result<()> {
        let store = load_cost_store(&self.store_path)?;
        self.records.extend(store.records);
        Ok(())
    }

    /// Save all records to disk.
    pub fn save(&self) -> crab_common::Result<()> {
        let store = CostStore {
            records: self.records.clone(),
        };
        save_cost_store(&self.store_path, &store)
    }
}

/// Per-provider aggregate summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub request_count: u64,
}

// ── On-disk format ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CostStore {
    records: Vec<UsageRecord>,
}

fn load_cost_store(path: &Path) -> crab_common::Result<CostStore> {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map_err(|e| {
            crab_common::Error::Config(format!("failed to parse {}: {e}", path.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CostStore::default()),
        Err(e) => Err(crab_common::Error::Config(format!(
            "failed to read {}: {e}",
            path.display()
        ))),
    }
}

fn save_cost_store(path: &Path, store: &CostStore) -> crab_common::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crab_common::Error::Config(format!("failed to create {}: {e}", parent.display()))
        })?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| crab_common::Error::Config(format!("failed to serialize costs: {e}")))?;
    std::fs::write(path, json)
        .map_err(|e| crab_common::Error::Config(format!("failed to write {}: {e}", path.display())))
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Crate-internal helper for other modules.
pub(crate) fn now_iso8601_pub() -> String {
    now_iso8601()
}

fn now_iso8601() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_timestamp(duration.as_secs())
}

fn format_unix_timestamp(secs: u64) -> String {
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cost_path() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crab-cost-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir.join("costs.json")
    }

    // ── ModelPricing ──────────────────────────────────────────────────

    #[test]
    fn model_pricing_calculate() {
        let p = ModelPricing::new(3.0, 15.0);
        // 1000 input + 500 output
        let cost = p.calculate(1000, 500);
        // (1000 * 3.0 + 500 * 15.0) / 1_000_000 = (3000 + 7500) / 1_000_000 = 0.0105
        assert!((cost - 0.0105).abs() < 1e-10);
    }

    #[test]
    fn model_pricing_zero_tokens() {
        let p = ModelPricing::new(3.0, 15.0);
        assert!((p.calculate(0, 0)).abs() < 1e-10);
    }

    #[test]
    fn model_pricing_serde_roundtrip() {
        let p = ModelPricing::new(2.5, 10.0);
        let json = serde_json::to_string(&p).unwrap();
        let parsed: ModelPricing = serde_json::from_str(&json).unwrap();
        assert_eq!(p, parsed);
    }

    // ── PricingTable ──────────────────────────────────────────────────

    #[test]
    fn pricing_table_defaults_have_entries() {
        let table = PricingTable::with_defaults();
        assert!(!table.models.is_empty());
        assert!(table.get("anthropic", "claude-sonnet-4-20250514").is_some());
        assert!(table.get("openai", "gpt-4o").is_some());
    }

    #[test]
    fn pricing_table_get_missing() {
        let table = PricingTable::default();
        assert!(table.get("unknown", "model").is_none());
    }

    #[test]
    fn pricing_table_set_and_get() {
        let mut table = PricingTable::default();
        table.set("deepseek", "chat", ModelPricing::new(0.14, 0.28));
        let pricing = table.get("deepseek", "chat").unwrap();
        assert!((pricing.input_per_million - 0.14).abs() < 1e-10);
    }

    #[test]
    fn pricing_table_serde_roundtrip() {
        let table = PricingTable::with_defaults();
        let json = serde_json::to_string_pretty(&table).unwrap();
        let parsed: PricingTable = serde_json::from_str(&json).unwrap();
        assert_eq!(table.models.len(), parsed.models.len());
    }

    // ── CostTracker ───────────────────────────────────────────────────

    #[test]
    fn record_and_query() {
        let mut tracker = CostTracker::new(PricingTable::with_defaults(), temp_cost_path());
        tracker.record("anthropic", "claude-sonnet-4-20250514", 1000, 500, None);
        tracker.record("openai", "gpt-4o", 2000, 1000, None);

        assert_eq!(tracker.records().len(), 2);
        assert!(tracker.total_cost() > 0.0);
    }

    #[test]
    fn record_unknown_model_zero_cost() {
        let mut tracker = CostTracker::new(PricingTable::default(), temp_cost_path());
        tracker.record("unknown", "model", 1000, 500, None);

        assert_eq!(tracker.records().len(), 1);
        assert!((tracker.records()[0].cost_usd).abs() < 1e-10);
    }

    #[test]
    fn total_tokens() {
        let mut tracker = CostTracker::new(PricingTable::default(), temp_cost_path());
        tracker.record("a", "m", 100, 50, None);
        tracker.record("b", "n", 200, 100, None);

        let (input, output) = tracker.total_tokens();
        assert_eq!(input, 300);
        assert_eq!(output, 150);
    }

    #[test]
    fn cost_by_provider() {
        let mut tracker = CostTracker::new(PricingTable::with_defaults(), temp_cost_path());
        tracker.record("anthropic", "claude-sonnet-4-20250514", 1000, 500, None);
        tracker.record("openai", "gpt-4o", 2000, 1000, None);

        let anthropic_cost = tracker.cost_by_provider("anthropic");
        let openai_cost = tracker.cost_by_provider("openai");
        assert!(anthropic_cost > 0.0);
        assert!(openai_cost > 0.0);
        assert!((tracker.total_cost() - anthropic_cost - openai_cost).abs() < 1e-10);
    }

    #[test]
    fn summary_by_provider() {
        let mut tracker = CostTracker::new(PricingTable::default(), temp_cost_path());
        tracker.record("anthropic", "m1", 100, 50, None);
        tracker.record("anthropic", "m2", 200, 100, None);
        tracker.record("openai", "m3", 300, 150, None);

        let summary = tracker.summary_by_provider();
        assert_eq!(summary.len(), 2);
        let ant = &summary["anthropic"];
        assert_eq!(ant.input_tokens, 300);
        assert_eq!(ant.output_tokens, 150);
        assert_eq!(ant.request_count, 2);
    }

    #[test]
    fn record_with_session_id() {
        let mut tracker = CostTracker::new(PricingTable::default(), temp_cost_path());
        tracker.record("a", "m", 100, 50, Some("session-1".to_string()));

        assert_eq!(
            tracker.records()[0].session_id.as_deref(),
            Some("session-1")
        );
    }

    // ── Persistence ───────────────────────────────────────────────────

    #[test]
    fn save_and_load_roundtrip() {
        let path = temp_cost_path();
        let mut tracker = CostTracker::new(PricingTable::with_defaults(), path.clone());
        tracker.record("anthropic", "claude-sonnet-4-20250514", 1000, 500, None);
        tracker.save().unwrap();

        let mut tracker2 = CostTracker::new(PricingTable::with_defaults(), path.clone());
        tracker2.load().unwrap();
        assert_eq!(tracker2.records().len(), 1);
        assert!((tracker2.total_cost() - tracker.total_cost()).abs() < 1e-10);

        let _ = std::fs::remove_file(&path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let mut tracker = CostTracker::new(
            PricingTable::default(),
            PathBuf::from("/nonexistent/costs.json"),
        );
        assert!(tracker.load().is_ok());
        assert!(tracker.records().is_empty());
    }

    #[test]
    fn save_creates_parent_dirs() {
        let path = temp_cost_path();
        let tracker = CostTracker::new(PricingTable::default(), path.clone());
        assert!(tracker.save().is_ok());
        assert!(path.exists());

        let _ = std::fs::remove_file(&path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }

    // ── Default path ──────────────────────────────────────────────────

    #[test]
    fn default_path_under_crab() {
        let path = CostTracker::default_path();
        assert!(path.ends_with("costs.json"));
        assert!(path.to_string_lossy().contains(".crab"));
    }

    // ── Debug impl ────────────────────────────────────────────────────

    #[test]
    fn debug_impl() {
        let tracker = CostTracker::new(PricingTable::default(), temp_cost_path());
        let debug = format!("{tracker:?}");
        assert!(debug.contains("CostTracker"));
    }

    // ── UsageRecord serde ─────────────────────────────────────────────

    #[test]
    fn usage_record_serde_roundtrip() {
        let record = UsageRecord {
            timestamp: "2026-04-05T12:00:00Z".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: 0.0105,
            session_id: Some("sess-1".to_string()),
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: UsageRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record, parsed);
    }

    #[test]
    fn usage_record_no_session_id() {
        let record = UsageRecord {
            timestamp: "2026-04-05T12:00:00Z".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.0,
            session_id: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains("session_id"));
    }
}
