//! Model capabilities and streaming token accumulation.
//!
//! `ModelCapabilities` describes what a model supports (vision, tool use, etc.).
//! `StreamingUsage` accumulates token usage from a series of `StreamEvent`s.

use crab_core::model::TokenUsage;
use serde::{Deserialize, Serialize};

use crate::types::StreamEvent;

/// Describes the capabilities of a specific model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ModelCapabilities {
    /// Model identifier (e.g. "claude-sonnet-4-20250514").
    pub model_id: String,
    /// Whether the model supports image/vision inputs.
    pub vision: bool,
    /// Whether the model supports tool use (function calling).
    pub tool_use: bool,
    /// Whether the model supports structured JSON output mode.
    pub json_mode: bool,
    /// Whether the model supports streaming responses.
    pub streaming: bool,
    /// Maximum output tokens the model can produce.
    pub max_output_tokens: u32,
    /// Maximum context window size in tokens.
    pub context_window: u32,
    /// Whether the model supports image inputs (PDF pages rendered as images).
    pub supports_images: bool,
    /// Whether the model supports PDF file inputs natively.
    pub supports_pdf: bool,
    /// Whether the model supports computer use (mouse/keyboard control).
    pub supports_computer_use: bool,
    /// Whether the model supports prompt caching.
    pub supports_caching: bool,
}

impl ModelCapabilities {
    /// Create capabilities for a known Anthropic model.
    #[must_use]
    pub fn anthropic(model_id: &str) -> Self {
        let (max_output, context) = match model_id {
            s if s.contains("opus") => (32_000, 200_000),
            s if s.contains("sonnet") => (16_000, 200_000),
            s if s.contains("haiku") => (8_192, 200_000),
            _ => (4_096, 200_000),
        };
        let supports_computer_use = model_id.contains("sonnet") || model_id.contains("opus");
        Self {
            model_id: model_id.to_string(),
            vision: true,
            tool_use: true,
            json_mode: false, // Anthropic uses tool_use for structured output
            streaming: true,
            max_output_tokens: max_output,
            context_window: context,
            supports_images: true,
            supports_pdf: true,
            supports_caching: true,
            supports_computer_use,
        }
    }

    /// Create capabilities for a known `OpenAI` model.
    #[must_use]
    pub fn openai(model_id: &str) -> Self {
        let (max_output, context, vision, json_mode) = match model_id {
            s if s.starts_with("gpt-4o") => (16_384, 128_000, true, true),
            s if s.starts_with("gpt-4-turbo") => (4_096, 128_000, true, true),
            s if s.starts_with("gpt-4") => (8_192, 8_192, false, false),
            s if s.starts_with("gpt-3.5") => (4_096, 16_385, false, true),
            s if s.starts_with("o1") || s.starts_with("o3") || s.starts_with("o4") => {
                (100_000, 200_000, true, true)
            }
            _ => (4_096, 128_000, false, true),
        };
        Self {
            model_id: model_id.to_string(),
            vision,
            tool_use: true,
            json_mode,
            streaming: true,
            max_output_tokens: max_output,
            context_window: context,
            supports_images: vision,
            supports_pdf: false,
            supports_caching: false,
            supports_computer_use: false,
        }
    }

    /// Create default/unknown capabilities.
    #[must_use]
    pub fn unknown(model_id: &str) -> Self {
        Self {
            model_id: model_id.to_string(),
            vision: false,
            tool_use: true,
            json_mode: false,
            streaming: true,
            max_output_tokens: 4_096,
            context_window: 128_000,
            supports_images: false,
            supports_pdf: false,
            supports_caching: false,
            supports_computer_use: false,
        }
    }
}

/// A capability that was requested but not supported by the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityWarning {
    /// Name of the unsupported capability.
    pub capability: String,
    /// Reason it is not available.
    pub reason: String,
}

impl std::fmt::Display for CapabilityWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.capability, self.reason)
    }
}

/// Capabilities after negotiation — what the model actually supports
/// given what was requested.
#[derive(Debug, Clone)]
pub struct NegotiatedCapabilities {
    /// The effective capabilities (trimmed to model support).
    pub effective: ModelCapabilities,
    /// Warnings for capabilities that were requested but not supported.
    pub warnings: Vec<CapabilityWarning>,
}

impl NegotiatedCapabilities {
    /// Whether any warnings were generated.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// Negotiate capabilities: trim requested features to what the model supports.
///
/// Returns `NegotiatedCapabilities` with the effective set and any warnings.
#[must_use]
pub fn negotiate_capabilities(
    requested: &RequestedCapabilities,
    model: &ModelCapabilities,
) -> NegotiatedCapabilities {
    let mut warnings = Vec::new();

    if requested.images && !model.supports_images {
        warnings.push(CapabilityWarning {
            capability: "images".to_string(),
            reason: format!("model {} does not support image inputs", model.model_id),
        });
    }
    if requested.pdf && !model.supports_pdf {
        warnings.push(CapabilityWarning {
            capability: "pdf".to_string(),
            reason: format!("model {} does not support PDF inputs", model.model_id),
        });
    }
    if requested.computer_use && !model.supports_computer_use {
        warnings.push(CapabilityWarning {
            capability: "computer_use".to_string(),
            reason: format!("model {} does not support computer use", model.model_id),
        });
    }
    if requested.caching && !model.supports_caching {
        warnings.push(CapabilityWarning {
            capability: "caching".to_string(),
            reason: format!("model {} does not support prompt caching", model.model_id),
        });
    }
    if requested.tool_use && !model.tool_use {
        warnings.push(CapabilityWarning {
            capability: "tool_use".to_string(),
            reason: format!("model {} does not support tool use", model.model_id),
        });
    }

    NegotiatedCapabilities {
        effective: model.clone(),
        warnings,
    }
}

/// What capabilities the caller requests.
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct RequestedCapabilities {
    pub images: bool,
    pub pdf: bool,
    pub computer_use: bool,
    pub caching: bool,
    pub tool_use: bool,
}

/// Accumulates token usage from streaming events.
///
/// Feed each `StreamEvent` into `update()` and retrieve the running total
/// via `usage()` at any time. Tracks both initial (`MessageStart`) and
/// final (`MessageDelta`) usage updates.
#[derive(Debug, Clone, Default)]
pub struct StreamingUsage {
    current: TokenUsage,
    stop_reason: Option<String>,
    message_id: Option<String>,
}

impl StreamingUsage {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a stream event, updating accumulated usage.
    ///
    /// Returns `true` if the event contained usage information.
    pub fn update(&mut self, event: &StreamEvent) -> bool {
        match event {
            StreamEvent::MessageStart { id, usage } => {
                self.message_id = Some(id.clone());
                self.current = usage.clone();
                true
            }
            StreamEvent::MessageDelta {
                usage, stop_reason, ..
            } => {
                // MessageDelta carries cumulative output tokens;
                // merge with existing input/cache tokens from MessageStart.
                self.current.output_tokens = usage.output_tokens;
                if usage.input_tokens > 0 {
                    self.current.input_tokens = usage.input_tokens;
                }
                self.stop_reason.clone_from(stop_reason);
                true
            }
            _ => false,
        }
    }

    /// Current accumulated token usage.
    #[must_use]
    pub fn usage(&self) -> &TokenUsage {
        &self.current
    }

    /// Consume and return final usage.
    #[must_use]
    pub fn into_usage(self) -> TokenUsage {
        self.current
    }

    /// Stop reason from the last `MessageDelta`, if any.
    #[must_use]
    pub fn stop_reason(&self) -> Option<&str> {
        self.stop_reason.as_deref()
    }

    /// Message ID from `MessageStart`.
    #[must_use]
    pub fn message_id(&self) -> Option<&str> {
        self.message_id.as_deref()
    }

    /// Total tokens consumed so far.
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.current.total()
    }
}

/// Model info returned by the list models API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier.
    pub id: String,
    /// Human-readable name (if available).
    pub name: Option<String>,
    /// Provider that owns this model.
    pub provider: String,
}

/// Result of a provider health check.
#[derive(Debug, Clone)]
pub struct HealthStatus {
    /// Whether the provider is reachable and the API key is valid.
    pub healthy: bool,
    /// Latency of the health check request.
    pub latency: std::time::Duration,
    /// Error message if unhealthy.
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── ModelCapabilities ───

    #[test]
    fn anthropic_opus_capabilities() {
        let caps = ModelCapabilities::anthropic("claude-opus-4-20250514");
        assert!(caps.vision);
        assert!(caps.tool_use);
        assert!(!caps.json_mode);
        assert!(caps.streaming);
        assert_eq!(caps.max_output_tokens, 32_000);
        assert_eq!(caps.context_window, 200_000);
    }

    #[test]
    fn anthropic_sonnet_capabilities() {
        let caps = ModelCapabilities::anthropic("claude-sonnet-4-20250514");
        assert_eq!(caps.max_output_tokens, 16_000);
        assert_eq!(caps.context_window, 200_000);
    }

    #[test]
    fn anthropic_haiku_capabilities() {
        let caps = ModelCapabilities::anthropic("claude-haiku-3-5");
        assert_eq!(caps.max_output_tokens, 8_192);
    }

    #[test]
    fn anthropic_unknown_model() {
        let caps = ModelCapabilities::anthropic("claude-future-99");
        assert_eq!(caps.max_output_tokens, 4_096);
    }

    #[test]
    fn openai_gpt4o_capabilities() {
        let caps = ModelCapabilities::openai("gpt-4o");
        assert!(caps.vision);
        assert!(caps.tool_use);
        assert!(caps.json_mode);
        assert_eq!(caps.max_output_tokens, 16_384);
        assert_eq!(caps.context_window, 128_000);
    }

    #[test]
    fn openai_gpt4_turbo_capabilities() {
        let caps = ModelCapabilities::openai("gpt-4-turbo-2024-04-09");
        assert!(caps.vision);
        assert!(caps.json_mode);
        assert_eq!(caps.max_output_tokens, 4_096);
    }

    #[test]
    fn openai_gpt4_base_capabilities() {
        let caps = ModelCapabilities::openai("gpt-4");
        assert!(!caps.vision);
        assert!(!caps.json_mode);
        assert_eq!(caps.context_window, 8_192);
    }

    #[test]
    fn openai_gpt35_capabilities() {
        let caps = ModelCapabilities::openai("gpt-3.5-turbo");
        assert!(!caps.vision);
        assert!(caps.json_mode);
        assert_eq!(caps.context_window, 16_385);
    }

    #[test]
    fn openai_o1_capabilities() {
        let caps = ModelCapabilities::openai("o1-preview");
        assert!(caps.vision);
        assert_eq!(caps.max_output_tokens, 100_000);
        assert_eq!(caps.context_window, 200_000);
    }

    #[test]
    fn openai_unknown_model() {
        let caps = ModelCapabilities::openai("some-custom-model");
        assert!(!caps.vision);
        assert!(caps.json_mode);
        assert_eq!(caps.max_output_tokens, 4_096);
    }

    #[test]
    fn unknown_capabilities() {
        let caps = ModelCapabilities::unknown("local-llama");
        assert!(!caps.vision);
        assert!(caps.tool_use);
        assert!(!caps.json_mode);
        assert_eq!(caps.max_output_tokens, 4_096);
    }

    #[test]
    fn capabilities_serde_roundtrip() {
        let caps = ModelCapabilities::anthropic("claude-sonnet-4-20250514");
        let json = serde_json::to_string(&caps).unwrap();
        let parsed: ModelCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(caps, parsed);
    }

    // ─── StreamingUsage ───

    #[test]
    fn streaming_usage_default_is_empty() {
        let su = StreamingUsage::new();
        assert!(su.usage().is_empty());
        assert_eq!(su.total_tokens(), 0);
        assert!(su.stop_reason().is_none());
        assert!(su.message_id().is_none());
    }

    #[test]
    fn streaming_usage_message_start() {
        let mut su = StreamingUsage::new();
        let event = StreamEvent::MessageStart {
            id: "msg_abc".into(),
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 0,
                cache_read_tokens: 50,
                cache_creation_tokens: 0,
            },
        };
        let updated = su.update(&event);
        assert!(updated);
        assert_eq!(su.usage().input_tokens, 100);
        assert_eq!(su.usage().cache_read_tokens, 50);
        assert_eq!(su.message_id(), Some("msg_abc"));
    }

    #[test]
    fn streaming_usage_message_delta() {
        let mut su = StreamingUsage::new();
        // First: MessageStart with input tokens
        su.update(&StreamEvent::MessageStart {
            id: "msg_1".into(),
            usage: TokenUsage {
                input_tokens: 200,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
        });
        // Then: MessageDelta with output tokens
        let updated = su.update(&StreamEvent::MessageDelta {
            usage: TokenUsage {
                input_tokens: 0,
                output_tokens: 150,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            stop_reason: Some("end_turn".into()),
        });
        assert!(updated);
        assert_eq!(su.usage().input_tokens, 200); // preserved from start
        assert_eq!(su.usage().output_tokens, 150);
        assert_eq!(su.total_tokens(), 350);
        assert_eq!(su.stop_reason(), Some("end_turn"));
    }

    #[test]
    fn streaming_usage_ignores_non_usage_events() {
        let mut su = StreamingUsage::new();
        assert!(!su.update(&StreamEvent::ContentDelta {
            index: 0,
            delta: "hello".into(),
        }));
        assert!(!su.update(&StreamEvent::ContentBlockStart {
            index: 0,
            content_type: "text".into(),
        }));
        assert!(!su.update(&StreamEvent::ContentBlockStop { index: 0 }));
        assert!(!su.update(&StreamEvent::MessageStop));
        assert!(!su.update(&StreamEvent::Error {
            message: "err".into(),
        }));
        assert!(su.usage().is_empty());
    }

    #[test]
    fn streaming_usage_into_usage() {
        let mut su = StreamingUsage::new();
        su.update(&StreamEvent::MessageStart {
            id: "m".into(),
            usage: TokenUsage {
                input_tokens: 42,
                output_tokens: 13,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
        });
        let usage = su.into_usage();
        assert_eq!(usage.input_tokens, 42);
        assert_eq!(usage.output_tokens, 13);
    }

    #[test]
    fn streaming_usage_multiple_deltas() {
        let mut su = StreamingUsage::new();
        su.update(&StreamEvent::MessageStart {
            id: "m".into(),
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
        });
        // Simulate incremental output token updates (cumulative)
        su.update(&StreamEvent::MessageDelta {
            usage: TokenUsage {
                input_tokens: 0,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            stop_reason: None,
        });
        su.update(&StreamEvent::MessageDelta {
            usage: TokenUsage {
                input_tokens: 0,
                output_tokens: 120,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            stop_reason: Some("end_turn".into()),
        });
        assert_eq!(su.usage().output_tokens, 120); // last delta wins (cumulative)
        assert_eq!(su.total_tokens(), 220);
    }

    // ─── ModelInfo ───

    #[test]
    fn model_info_serde_roundtrip() {
        let info = ModelInfo {
            id: "claude-sonnet-4-20250514".into(),
            name: Some("Claude Sonnet 4".into()),
            provider: "anthropic".into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: ModelInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, parsed);
    }

    #[test]
    fn model_info_without_name() {
        let info = ModelInfo {
            id: "local-model".into(),
            name: None,
            provider: "ollama".into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":null"));
    }

    // ─── HealthStatus ───

    #[test]
    fn health_status_healthy() {
        let status = HealthStatus {
            healthy: true,
            latency: std::time::Duration::from_millis(150),
            error: None,
        };
        assert!(status.healthy);
        assert!(status.error.is_none());
    }

    #[test]
    fn health_status_unhealthy() {
        let status = HealthStatus {
            healthy: false,
            latency: std::time::Duration::from_secs(5),
            error: Some("connection refused".into()),
        };
        assert!(!status.healthy);
        assert_eq!(status.error.as_deref(), Some("connection refused"));
    }

    // ─── New capability fields ───

    #[test]
    fn anthropic_sonnet_supports_computer_use() {
        let caps = ModelCapabilities::anthropic("claude-sonnet-4-20250514");
        assert!(caps.supports_images);
        assert!(caps.supports_pdf);
        assert!(caps.supports_caching);
        assert!(caps.supports_computer_use);
    }

    #[test]
    fn anthropic_haiku_no_computer_use() {
        let caps = ModelCapabilities::anthropic("claude-haiku-3-5");
        assert!(caps.supports_images);
        assert!(!caps.supports_computer_use);
    }

    #[test]
    fn openai_no_pdf_or_caching() {
        let caps = ModelCapabilities::openai("gpt-4o");
        assert!(caps.supports_images);
        assert!(!caps.supports_pdf);
        assert!(!caps.supports_caching);
        assert!(!caps.supports_computer_use);
    }

    #[test]
    fn unknown_no_extended_capabilities() {
        let caps = ModelCapabilities::unknown("local-llama");
        assert!(!caps.supports_images);
        assert!(!caps.supports_pdf);
        assert!(!caps.supports_caching);
        assert!(!caps.supports_computer_use);
    }

    // ─── Capability negotiation ───

    #[test]
    fn negotiate_no_warnings_when_all_supported() {
        let model = ModelCapabilities::anthropic("claude-sonnet-4-20250514");
        let requested = RequestedCapabilities {
            images: true,
            pdf: true,
            computer_use: true,
            caching: true,
            tool_use: true,
        };
        let result = negotiate_capabilities(&requested, &model);
        assert!(!result.has_warnings());
    }

    #[test]
    fn negotiate_warns_on_unsupported() {
        let model = ModelCapabilities::openai("gpt-4o");
        let requested = RequestedCapabilities {
            images: true,
            pdf: true,
            computer_use: true,
            caching: true,
            tool_use: true,
        };
        let result = negotiate_capabilities(&requested, &model);
        assert!(result.has_warnings());
        let caps: Vec<&str> = result
            .warnings
            .iter()
            .map(|w| w.capability.as_str())
            .collect();
        assert!(caps.contains(&"pdf"));
        assert!(caps.contains(&"computer_use"));
        assert!(caps.contains(&"caching"));
        assert!(!caps.contains(&"images")); // gpt-4o supports images
    }

    #[test]
    fn negotiate_no_warnings_when_nothing_requested() {
        let model = ModelCapabilities::unknown("local");
        let requested = RequestedCapabilities::default();
        let result = negotiate_capabilities(&requested, &model);
        assert!(!result.has_warnings());
    }

    #[test]
    fn capability_warning_display() {
        let w = CapabilityWarning {
            capability: "pdf".to_string(),
            reason: "not supported".to_string(),
        };
        assert_eq!(w.to_string(), "pdf: not supported");
    }
}
