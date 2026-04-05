//! MCP sampling support — `sampling/createMessage`.
//!
//! Allows MCP servers to request LLM completions from the host application
//! via the [`SamplingHandler`] trait.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Role in a sampling message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SamplingRole {
    User,
    Assistant,
}

impl std::fmt::Display for SamplingRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Assistant => write!(f, "assistant"),
        }
    }
}

/// Content within a sampling message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SamplingContent {
    Text { text: String },
    Image { data: String, mime_type: String },
}

impl SamplingContent {
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    #[must_use]
    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self::Image {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            Self::Image { .. } => None,
        }
    }
}

/// A single message in a sampling request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingMessage {
    pub role: SamplingRole,
    pub content: SamplingContent,
}

impl SamplingMessage {
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: SamplingRole::User,
            content: SamplingContent::text(text),
        }
    }

    #[must_use]
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: SamplingRole::Assistant,
            content: SamplingContent::text(text),
        }
    }
}

/// Model preferences for sampling.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPreferences {
    /// Hints about which models to prefer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<ModelHint>,
    /// Cost priority (0.0 = don't care, 1.0 = minimize cost).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_priority: Option<f64>,
    /// Speed priority (0.0 = don't care, 1.0 = maximize speed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_priority: Option<f64>,
    /// Intelligence priority (0.0 = don't care, 1.0 = maximize quality).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intelligence_priority: Option<f64>,
}

/// A hint about a preferred model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelHint {
    /// Model name pattern (e.g. "claude-3", "gpt-4").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Stop reason for a sampling response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    StopSequence,
    MaxTokens,
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EndTurn => write!(f, "end_turn"),
            Self::StopSequence => write!(f, "stop_sequence"),
            Self::MaxTokens => write!(f, "max_tokens"),
        }
    }
}

/// Request for `sampling/createMessage`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplingRequest {
    pub messages: Vec<SamplingMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_preferences: Option<ModelPreferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
    /// Opaque metadata from the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl SamplingRequest {
    /// Create a simple text request.
    #[must_use]
    pub fn simple(prompt: impl Into<String>) -> Self {
        Self {
            messages: vec![SamplingMessage::user(prompt)],
            model_preferences: None,
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            stop_sequences: Vec::new(),
            metadata: None,
        }
    }

    /// Set the system prompt.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set max tokens.
    #[must_use]
    pub fn with_max_tokens(mut self, max: u64) -> Self {
        self.max_tokens = Some(max);
        self
    }
}

/// Response for `sampling/createMessage`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplingResponse {
    pub role: SamplingRole,
    pub content: SamplingContent,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
}

impl SamplingResponse {
    /// Create a text response.
    #[must_use]
    pub fn text(model: impl Into<String>, text: impl Into<String>, stop_reason: StopReason) -> Self {
        Self {
            role: SamplingRole::Assistant,
            content: SamplingContent::text(text),
            model: model.into(),
            stop_reason: Some(stop_reason),
        }
    }
}

/// Trait for handling sampling requests from MCP servers.
///
/// The host application implements this to provide LLM completion capability
/// when servers request `sampling/createMessage`.
pub trait SamplingHandler: Send + Sync {
    /// Process a sampling request and return the model response.
    fn create_message(&self, request: &SamplingRequest) -> Result<SamplingResponse, String>;
}

/// A simple handler that returns a fixed response (useful for testing).
pub struct FixedSamplingHandler {
    pub model: String,
    pub response_text: String,
}

impl FixedSamplingHandler {
    #[must_use]
    pub fn new(model: impl Into<String>, response_text: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response_text: response_text.into(),
        }
    }
}

impl SamplingHandler for FixedSamplingHandler {
    fn create_message(&self, _request: &SamplingRequest) -> Result<SamplingResponse, String> {
        Ok(SamplingResponse::text(
            &self.model,
            &self.response_text,
            StopReason::EndTurn,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_role_display_and_serde() {
        assert_eq!(SamplingRole::User.to_string(), "user");
        assert_eq!(SamplingRole::Assistant.to_string(), "assistant");
        let json = serde_json::to_string(&SamplingRole::User).unwrap();
        let back: SamplingRole = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SamplingRole::User);
    }

    #[test]
    fn sampling_content_text() {
        let c = SamplingContent::text("hello");
        assert_eq!(c.as_text(), Some("hello"));
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"type\":\"text\""));
    }

    #[test]
    fn sampling_content_image() {
        let c = SamplingContent::image("base64data", "image/png");
        assert!(c.as_text().is_none());
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"type\":\"image\""));
    }

    #[test]
    fn sampling_message_constructors() {
        let u = SamplingMessage::user("hi");
        assert_eq!(u.role, SamplingRole::User);
        let a = SamplingMessage::assistant("hello");
        assert_eq!(a.role, SamplingRole::Assistant);
    }

    #[test]
    fn stop_reason_display_and_serde() {
        for (sr, label) in [
            (StopReason::EndTurn, "end_turn"),
            (StopReason::StopSequence, "stop_sequence"),
            (StopReason::MaxTokens, "max_tokens"),
        ] {
            assert_eq!(sr.to_string(), label);
            let json = serde_json::to_string(&sr).unwrap();
            let back: StopReason = serde_json::from_str(&json).unwrap();
            assert_eq!(sr, back);
        }
    }

    #[test]
    fn sampling_request_simple() {
        let req = SamplingRequest::simple("What is 2+2?");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, SamplingRole::User);
        assert!(req.system_prompt.is_none());
        assert!(req.max_tokens.is_none());
    }

    #[test]
    fn sampling_request_builder() {
        let req = SamplingRequest::simple("prompt")
            .with_system_prompt("You are helpful")
            .with_max_tokens(100);
        assert_eq!(req.system_prompt.as_deref(), Some("You are helpful"));
        assert_eq!(req.max_tokens, Some(100));
    }

    #[test]
    fn sampling_request_serde_roundtrip() {
        let req = SamplingRequest {
            messages: vec![SamplingMessage::user("hi")],
            model_preferences: Some(ModelPreferences {
                hints: vec![ModelHint { name: Some("claude-3".into()) }],
                cost_priority: Some(0.5),
                speed_priority: None,
                intelligence_priority: Some(0.8),
            }),
            system_prompt: Some("sys".into()),
            max_tokens: Some(200),
            temperature: Some(0.7),
            stop_sequences: vec!["STOP".into()],
            metadata: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: SamplingRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.messages.len(), 1);
        assert_eq!(back.max_tokens, Some(200));
        assert_eq!(back.temperature, Some(0.7));
        let prefs = back.model_preferences.unwrap();
        assert_eq!(prefs.hints[0].name.as_deref(), Some("claude-3"));
    }

    #[test]
    fn sampling_response_text() {
        let resp = SamplingResponse::text("claude-3", "The answer is 4", StopReason::EndTurn);
        assert_eq!(resp.model, "claude-3");
        assert_eq!(resp.role, SamplingRole::Assistant);
        assert_eq!(resp.content.as_text(), Some("The answer is 4"));
        assert_eq!(resp.stop_reason, Some(StopReason::EndTurn));
    }

    #[test]
    fn sampling_response_serde_roundtrip() {
        let resp = SamplingResponse::text("gpt-4", "answer", StopReason::MaxTokens);
        let json = serde_json::to_string(&resp).unwrap();
        let back: SamplingResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.model, "gpt-4");
        assert_eq!(back.stop_reason, Some(StopReason::MaxTokens));
    }

    #[test]
    fn fixed_sampling_handler() {
        let handler = FixedSamplingHandler::new("test-model", "fixed response");
        let req = SamplingRequest::simple("anything");
        let resp = handler.create_message(&req).unwrap();
        assert_eq!(resp.model, "test-model");
        assert_eq!(resp.content.as_text(), Some("fixed response"));
        assert_eq!(resp.stop_reason, Some(StopReason::EndTurn));
    }

    #[test]
    fn model_preferences_default() {
        let prefs = ModelPreferences::default();
        assert!(prefs.hints.is_empty());
        assert!(prefs.cost_priority.is_none());
        assert!(prefs.speed_priority.is_none());
        assert!(prefs.intelligence_priority.is_none());
    }
}
