//! Conversation quality evaluation.
//!
//! Scores a conversation on three dimensions — completeness, coherence,
//! and efficiency — and produces an overall [`QualityScore`].

use crab_core::message::{ContentBlock, Message, Role};
use serde::Serialize;

// ── Score ───────────────────────────────────────────────────────────

/// Quality score for a conversation session. Each dimension is 0.0–1.0.
#[derive(Debug, Clone, Serialize)]
pub struct QualityScore {
    /// Were user requests fulfilled? (tool success rate)
    pub completeness: f64,
    /// Is the conversation coherent? (adjacent message relevance)
    pub coherence: f64,
    /// Token efficiency (effective output / total tokens).
    pub efficiency: f64,
    /// Weighted average of the three dimensions.
    pub overall: f64,
}

impl QualityScore {
    /// Create a score, clamping all values to 0.0–1.0 and computing overall.
    #[must_use]
    pub fn new(completeness: f64, coherence: f64, efficiency: f64) -> Self {
        let completeness = completeness.clamp(0.0, 1.0);
        let coherence = coherence.clamp(0.0, 1.0);
        let efficiency = efficiency.clamp(0.0, 1.0);
        let overall = completeness * 0.4 + coherence * 0.3 + efficiency * 0.3;
        Self {
            completeness,
            coherence,
            efficiency,
            overall,
        }
    }
}

// ── Evaluation ──────────────────────────────────────────────────────

/// Evaluate a conversation and return a [`QualityScore`].
#[must_use]
pub fn evaluate_session(messages: &[Message]) -> QualityScore {
    let completeness = evaluate_completeness(messages);
    let coherence = evaluate_coherence(messages);
    let efficiency = evaluate_efficiency(messages);
    QualityScore::new(completeness, coherence, efficiency)
}

/// Completeness: ratio of successful tool results to total tool results.
///
/// If there are no tool calls, completeness is 1.0 (nothing to fail).
#[allow(clippy::cast_precision_loss)]
fn evaluate_completeness(messages: &[Message]) -> f64 {
    let mut total_results = 0u64;
    let mut success_results = 0u64;

    for msg in messages {
        for block in &msg.content {
            if let ContentBlock::ToolResult { is_error, .. } = block {
                total_results += 1;
                if !is_error {
                    success_results += 1;
                }
            }
        }
    }

    if total_results == 0 {
        1.0
    } else {
        success_results as f64 / total_results as f64
    }
}

/// Coherence: measures whether the conversation follows a logical pattern.
///
/// Checks that user messages are followed by assistant responses and that
/// adjacent messages share some keyword overlap. Returns 0.0–1.0.
#[allow(clippy::cast_precision_loss)]
fn evaluate_coherence(messages: &[Message]) -> f64 {
    if messages.len() < 2 {
        return 1.0;
    }

    let mut pair_count = 0u64;
    let mut coherent_pairs = 0u64;

    for window in messages.windows(2) {
        let a = &window[0];
        let b = &window[1];

        // Only check user→assistant or assistant→user transitions
        let is_transition = (a.role == Role::User && b.role == Role::Assistant)
            || (a.role == Role::Assistant && b.role == Role::User);

        if !is_transition {
            continue;
        }

        pair_count += 1;

        let text_a = a.text().to_lowercase();
        let text_b = b.text().to_lowercase();

        // Simple coherence: the response should share at least one
        // substantive word (>= 4 chars) with the prompt.
        let words_a: std::collections::HashSet<&str> =
            text_a.split_whitespace().filter(|w| w.len() >= 4).collect();
        let words_b: std::collections::HashSet<&str> =
            text_b.split_whitespace().filter(|w| w.len() >= 4).collect();

        if words_a.is_empty() || words_b.is_empty() || !words_a.is_disjoint(&words_b) {
            coherent_pairs += 1;
        }
    }

    if pair_count == 0 {
        1.0
    } else {
        coherent_pairs as f64 / pair_count as f64
    }
}

/// Efficiency: ratio of assistant text tokens to total tokens.
///
/// A high ratio means most tokens were used for delivering answers, not
#[allow(clippy::cast_precision_loss)]
/// overhead. If there are no tokens, returns 1.0.
fn evaluate_efficiency(messages: &[Message]) -> f64 {
    let mut total_tokens = 0u64;
    let mut assistant_text_tokens = 0u64;

    for msg in messages {
        let tokens = msg.estimated_tokens();
        total_tokens += tokens;
        if msg.role == Role::Assistant {
            // Count only text blocks (not tool use overhead)
            let text_chars: usize = msg
                .content
                .iter()
                .filter_map(|b| b.as_text())
                .map(str::len)
                .sum();
            assistant_text_tokens += text_chars as u64 / 4 + 1;
        }
    }

    if total_tokens == 0 {
        1.0
    } else {
        (assistant_text_tokens as f64 / total_tokens as f64).min(1.0)
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_msg(text: &str) -> Message {
        Message::user(text)
    }

    fn assistant_msg(text: &str) -> Message {
        Message::assistant(text)
    }

    fn tool_result_msg(is_error: bool) -> Message {
        Message::tool_result("id1", "result content", is_error)
    }

    fn tool_use_assistant(name: &str) -> Message {
        Message::new(
            Role::Assistant,
            vec![ContentBlock::tool_use("id1", name, json!({}))],
        )
    }

    // ── completeness ────────────────────────────────────────

    #[test]
    fn completeness_no_tools() {
        let score = evaluate_completeness(&[user_msg("hi"), assistant_msg("hello")]);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn completeness_all_success() {
        let msgs = [tool_result_msg(false), tool_result_msg(false)];
        let score = evaluate_completeness(&msgs);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn completeness_half_errors() {
        let msgs = [tool_result_msg(false), tool_result_msg(true)];
        let score = evaluate_completeness(&msgs);
        assert!((score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn completeness_all_errors() {
        let msgs = [tool_result_msg(true), tool_result_msg(true)];
        let score = evaluate_completeness(&msgs);
        assert!(score.abs() < f64::EPSILON);
    }

    // ── coherence ───────────────────────────────────────────

    #[test]
    fn coherence_empty() {
        assert!((evaluate_coherence(&[]) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn coherence_single_message() {
        assert!((evaluate_coherence(&[user_msg("hi")]) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn coherence_related_messages() {
        let msgs = [
            user_msg("explain the authentication system"),
            assistant_msg("the authentication system uses JWT tokens"),
        ];
        let score = evaluate_coherence(&msgs);
        assert!(score > 0.5); // shares "authentication" and "system"
    }

    #[test]
    fn coherence_unrelated_messages() {
        let msgs = [
            user_msg("explain quantum physics thoroughly"),
            assistant_msg("the recipe calls for butter and sugar"),
        ];
        let score = evaluate_coherence(&msgs);
        // No overlapping words >= 4 chars
        assert!(score < 1.0);
    }

    // ── efficiency ──────────────────────────────────────────

    #[test]
    fn efficiency_empty() {
        assert!((evaluate_efficiency(&[]) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn efficiency_pure_assistant() {
        let msgs = [assistant_msg(
            "a long detailed response about the topic at hand",
        )];
        let score = evaluate_efficiency(&msgs);
        assert!(score > 0.5);
    }

    #[test]
    fn efficiency_balanced() {
        let msgs = [
            user_msg("explain Rust ownership in great detail please"),
            assistant_msg("Rust ownership ensures memory safety without garbage collection"),
        ];
        let score = evaluate_efficiency(&msgs);
        assert!(score > 0.0);
        assert!(score <= 1.0);
    }

    // ── QualityScore ────────────────────────────────────────

    #[test]
    fn quality_score_clamping() {
        let score = QualityScore::new(1.5, -0.5, 0.5);
        assert!((score.completeness - 1.0).abs() < f64::EPSILON);
        assert!(score.coherence.abs() < f64::EPSILON);
        assert!((score.efficiency - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn quality_score_overall_weighted() {
        let score = QualityScore::new(1.0, 1.0, 1.0);
        assert!((score.overall - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn quality_score_overall_zero() {
        let score = QualityScore::new(0.0, 0.0, 0.0);
        assert!(score.overall.abs() < f64::EPSILON);
    }

    #[test]
    fn quality_score_serializes() {
        let score = QualityScore::new(0.8, 0.9, 0.7);
        let json = serde_json::to_string(&score).unwrap();
        assert!(json.contains("completeness"));
        assert!(json.contains("coherence"));
        assert!(json.contains("efficiency"));
        assert!(json.contains("overall"));
    }

    // ── evaluate_session (integration) ──────────────────────

    #[test]
    fn evaluate_empty() {
        let score = evaluate_session(&[]);
        assert!((score.completeness - 1.0).abs() < f64::EPSILON);
        assert!((score.coherence - 1.0).abs() < f64::EPSILON);
        assert!((score.efficiency - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn evaluate_simple_conversation() {
        let msgs = [
            user_msg("what is Rust programming language"),
            assistant_msg("Rust is a systems programming language"),
        ];
        let score = evaluate_session(&msgs);
        assert!(score.completeness > 0.5);
        assert!(score.coherence > 0.5);
        assert!(score.overall > 0.0);
        assert!(score.overall <= 1.0);
    }

    #[test]
    fn evaluate_with_tool_errors() {
        let msgs = [
            user_msg("read the file"),
            tool_use_assistant("read_file"),
            tool_result_msg(true),
            assistant_msg("error reading file"),
            user_msg("try again"),
            tool_use_assistant("read_file"),
            tool_result_msg(false),
            assistant_msg("here is the file content"),
        ];
        let score = evaluate_session(&msgs);
        assert!((score.completeness - 0.5).abs() < f64::EPSILON); // 1/2 success
    }
}
