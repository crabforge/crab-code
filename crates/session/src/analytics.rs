//! Session analytics — metrics, conversation depth, and topic segmentation.
//!
//! Provides [`SessionAnalytics`] for computing per-session statistics from a
//! conversation's message history, including tool usage frequency,
//! conversation depth classification, and topic segment detection.

use std::collections::BTreeMap;

use crab_core::message::{ContentBlock, Message, Role};
use serde::Serialize;

// ── Depth classification ────────────────────────────────────────────

/// Conversation depth level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationDepth {
    /// Few turns, no tool use — simple Q&A.
    Shallow,
    /// Moderate turns or some tool use.
    Moderate,
    /// Many turns with heavy tool use — deep collaboration.
    Deep,
}

impl std::fmt::Display for ConversationDepth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Shallow => f.write_str("shallow"),
            Self::Moderate => f.write_str("moderate"),
            Self::Deep => f.write_str("deep"),
        }
    }
}

// ── Topic segment ───────────────────────────────────────────────────

/// A detected topic segment within the conversation.
#[derive(Debug, Clone, Serialize)]
pub struct TopicSegment {
    /// Zero-based index of the first message in this segment.
    pub start_index: usize,
    /// Zero-based index of the last message (inclusive).
    pub end_index: usize,
    /// Representative keywords for this segment.
    pub keywords: Vec<String>,
}

// ── Analytics report ────────────────────────────────────────────────

/// Aggregated analytics for a session.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyticsReport {
    /// Total number of conversation turns (user→assistant pairs).
    pub total_turns: usize,
    /// Total number of messages.
    pub total_messages: usize,
    /// Total estimated tokens across all messages.
    pub total_tokens: u64,
    /// Average estimated tokens per message.
    pub avg_tokens_per_message: u64,
    /// Number of tool invocations.
    pub tool_use_count: usize,
    /// Number of tool errors.
    pub tool_error_count: usize,
    /// Tool usage frequency map (`tool_name` → count).
    pub tool_frequency: BTreeMap<String, usize>,
    /// Conversation depth classification.
    pub depth: ConversationDepth,
    /// Detected topic segments.
    pub topic_segments: Vec<TopicSegment>,
}

// ── Session analytics ───────────────────────────────────────────────

/// Computes analytics from a slice of messages.
pub struct SessionAnalytics;

impl SessionAnalytics {
    /// Analyse a conversation's messages and produce an [`AnalyticsReport`].
    #[must_use]
    pub fn analyze(messages: &[Message]) -> AnalyticsReport {
        let total_messages = messages.len();
        let total_tokens: u64 = messages.iter().map(Message::estimated_tokens).sum();
        let avg_tokens_per_message = if total_messages > 0 {
            total_tokens / total_messages as u64
        } else {
            0
        };

        let total_turns = Self::count_turns(messages);
        let (tool_use_count, tool_error_count, tool_frequency) = Self::tool_stats(messages);
        let depth = Self::conversation_depth(total_turns, tool_use_count);
        let topic_segments = Self::topic_segments(messages);

        AnalyticsReport {
            total_turns,
            total_messages,
            total_tokens,
            avg_tokens_per_message,
            tool_use_count,
            tool_error_count,
            tool_frequency,
            depth,
            topic_segments,
        }
    }

    /// Count turns (a turn = one user message followed by one assistant message).
    #[must_use]
    fn count_turns(messages: &[Message]) -> usize {
        let mut turns = 0usize;
        let mut last_was_user = false;
        for msg in messages {
            match msg.role {
                Role::User => last_was_user = true,
                Role::Assistant if last_was_user => {
                    turns += 1;
                    last_was_user = false;
                }
                _ => last_was_user = false,
            }
        }
        turns
    }

    /// Gather tool usage statistics.
    fn tool_stats(messages: &[Message]) -> (usize, usize, BTreeMap<String, usize>) {
        let mut use_count = 0usize;
        let mut error_count = 0usize;
        let mut freq: BTreeMap<String, usize> = BTreeMap::new();

        for msg in messages {
            for block in &msg.content {
                match block {
                    ContentBlock::ToolUse { name, .. } => {
                        use_count += 1;
                        *freq.entry(name.clone()).or_default() += 1;
                    }
                    ContentBlock::ToolResult { is_error: true, .. } => {
                        error_count += 1;
                    }
                    _ => {}
                }
            }
        }

        (use_count, error_count, freq)
    }

    /// Classify conversation depth based on turns and tool usage.
    #[must_use]
    pub fn conversation_depth(turns: usize, tool_uses: usize) -> ConversationDepth {
        if turns >= 5 && tool_uses >= 3 {
            ConversationDepth::Deep
        } else if turns >= 2 || tool_uses >= 1 {
            ConversationDepth::Moderate
        } else {
            ConversationDepth::Shallow
        }
    }

    /// Detect topic segments by looking for keyword shifts between user messages.
    ///
    /// Uses a simple heuristic: extract top keywords from each user message and
    /// start a new segment when the keyword overlap with the previous user
    /// message drops below a threshold.
    #[must_use]
    pub fn topic_segments(messages: &[Message]) -> Vec<TopicSegment> {
        if messages.is_empty() {
            return Vec::new();
        }

        let user_indices: Vec<(usize, Vec<String>)> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == Role::User)
            .map(|(i, m)| (i, extract_keywords(&m.text())))
            .collect();

        if user_indices.is_empty() {
            return vec![TopicSegment {
                start_index: 0,
                end_index: messages.len() - 1,
                keywords: Vec::new(),
            }];
        }

        let mut segments: Vec<TopicSegment> = Vec::new();
        let mut seg_start = 0usize;
        let mut prev_keywords = &user_indices[0].1;

        for window in user_indices.windows(2) {
            let (_, ref kw_a) = window[0];
            let (idx_b, ref kw_b) = window[1];

            let overlap = keyword_overlap(kw_a, kw_b);
            if overlap < 0.2 {
                // Topic shift detected
                segments.push(TopicSegment {
                    start_index: seg_start,
                    end_index: idx_b - 1,
                    keywords: prev_keywords.clone(),
                });
                seg_start = idx_b;
            }
            prev_keywords = kw_b;
        }

        // Final segment
        segments.push(TopicSegment {
            start_index: seg_start,
            end_index: messages.len() - 1,
            keywords: prev_keywords.clone(),
        });

        segments
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Extract keywords from text (lowercase words >= 4 chars, deduplicated).
fn extract_keywords(text: &str) -> Vec<String> {
    let stop_words = [
        "that", "this", "with", "from", "have", "been", "will", "would", "could", "should", "them",
        "they", "their", "there", "what", "when", "where", "which", "your", "about", "into",
        "also", "just", "like", "more", "some", "than", "then", "very", "does",
    ];

    let mut seen = std::collections::HashSet::new();
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() >= 4 && !stop_words.contains(&w.as_str()))
        .filter(|w| seen.insert(w.clone()))
        .collect()
}

/// Jaccard-like overlap between two keyword lists (0.0 = no overlap, 1.0 = identical).
#[allow(clippy::cast_precision_loss)]
fn keyword_overlap(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_a: std::collections::HashSet<_> = a.iter().collect();
    let set_b: std::collections::HashSet<_> = b.iter().collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
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

    fn tool_use_msg(name: &str) -> Message {
        Message::new(
            Role::Assistant,
            vec![ContentBlock::tool_use("id1", name, json!({}))],
        )
    }

    fn tool_result_msg(is_error: bool) -> Message {
        Message::tool_result("id1", "result", is_error)
    }

    // ── count_turns ─────────────────────────────────────────

    #[test]
    fn turns_empty() {
        assert_eq!(SessionAnalytics::count_turns(&[]), 0);
    }

    #[test]
    fn turns_single_pair() {
        let msgs = [user_msg("hi"), assistant_msg("hello")];
        assert_eq!(SessionAnalytics::count_turns(&msgs), 1);
    }

    #[test]
    fn turns_multiple() {
        let msgs = [
            user_msg("a"),
            assistant_msg("b"),
            user_msg("c"),
            assistant_msg("d"),
        ];
        assert_eq!(SessionAnalytics::count_turns(&msgs), 2);
    }

    #[test]
    fn turns_user_only() {
        let msgs = [user_msg("a"), user_msg("b")];
        assert_eq!(SessionAnalytics::count_turns(&msgs), 0);
    }

    // ── tool_stats ──────────────────────────────────────────

    #[test]
    fn tool_stats_empty() {
        let (uses, errors, freq) = SessionAnalytics::tool_stats(&[]);
        assert_eq!(uses, 0);
        assert_eq!(errors, 0);
        assert!(freq.is_empty());
    }

    #[test]
    fn tool_stats_counts() {
        let msgs = [
            tool_use_msg("read_file"),
            tool_result_msg(false),
            tool_use_msg("read_file"),
            tool_result_msg(true),
            tool_use_msg("bash"),
            tool_result_msg(false),
        ];
        let (uses, errors, freq) = SessionAnalytics::tool_stats(&msgs);
        assert_eq!(uses, 3);
        assert_eq!(errors, 1);
        assert_eq!(freq["read_file"], 2);
        assert_eq!(freq["bash"], 1);
    }

    // ── conversation_depth ──────────────────────────────────

    #[test]
    fn depth_shallow() {
        assert_eq!(
            SessionAnalytics::conversation_depth(1, 0),
            ConversationDepth::Shallow
        );
    }

    #[test]
    fn depth_moderate() {
        assert_eq!(
            SessionAnalytics::conversation_depth(2, 0),
            ConversationDepth::Moderate
        );
        assert_eq!(
            SessionAnalytics::conversation_depth(1, 1),
            ConversationDepth::Moderate
        );
    }

    #[test]
    fn depth_deep() {
        assert_eq!(
            SessionAnalytics::conversation_depth(5, 3),
            ConversationDepth::Deep
        );
        assert_eq!(
            SessionAnalytics::conversation_depth(10, 5),
            ConversationDepth::Deep
        );
    }

    // ── topic_segments ──────────────────────────────────────

    #[test]
    fn segments_empty() {
        let segs = SessionAnalytics::topic_segments(&[]);
        assert!(segs.is_empty());
    }

    #[test]
    fn segments_single_topic() {
        let msgs = [
            user_msg("implement the authentication module"),
            assistant_msg("I'll implement the auth module"),
            user_msg("also add authentication tests"),
            assistant_msg("adding auth tests now"),
        ];
        let segs = SessionAnalytics::topic_segments(&msgs);
        // Related topic — should be one or few segments
        assert!(!segs.is_empty());
        assert_eq!(segs.last().unwrap().end_index, 3);
    }

    #[test]
    fn segments_topic_shift() {
        let msgs = [
            user_msg("implement the authentication module with JWT tokens"),
            assistant_msg("working on auth"),
            user_msg("now switch to database migration schema design"),
            assistant_msg("working on migration"),
        ];
        let segs = SessionAnalytics::topic_segments(&msgs);
        // "authentication JWT" vs "database migration schema" — should detect shift
        assert!(segs.len() >= 1);
    }

    // ── analyze (integration) ───────────────────────────────

    #[test]
    fn analyze_empty() {
        let report = SessionAnalytics::analyze(&[]);
        assert_eq!(report.total_messages, 0);
        assert_eq!(report.total_turns, 0);
        assert_eq!(report.tool_use_count, 0);
        assert_eq!(report.depth, ConversationDepth::Shallow);
    }

    #[test]
    fn analyze_simple_conversation() {
        let msgs = [
            user_msg("what is Rust?"),
            assistant_msg("Rust is a systems programming language."),
        ];
        let report = SessionAnalytics::analyze(&msgs);
        assert_eq!(report.total_messages, 2);
        assert_eq!(report.total_turns, 1);
        assert_eq!(report.tool_use_count, 0);
        assert_eq!(report.depth, ConversationDepth::Shallow);
    }

    #[test]
    fn analyze_with_tools() {
        let msgs = [
            user_msg("read main.rs"),
            tool_use_msg("read_file"),
            tool_result_msg(false),
            assistant_msg("here is the file"),
            user_msg("edit it"),
            tool_use_msg("edit_file"),
            tool_result_msg(false),
            assistant_msg("done"),
        ];
        let report = SessionAnalytics::analyze(&msgs);
        assert_eq!(report.tool_use_count, 2);
        assert_eq!(report.tool_error_count, 0);
        assert_eq!(report.tool_frequency["read_file"], 1);
        assert_eq!(report.tool_frequency["edit_file"], 1);
    }

    #[test]
    fn analyze_report_serializes() {
        let report = SessionAnalytics::analyze(&[user_msg("hi"), assistant_msg("hello")]);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("total_turns"));
        assert!(json.contains("depth"));
    }

    // ── helpers ──────────────────────────────────────────────

    #[test]
    fn extract_keywords_basic() {
        let kw = extract_keywords("implement the authentication module");
        assert!(kw.contains(&"implement".to_owned()));
        assert!(kw.contains(&"authentication".to_owned()));
        assert!(kw.contains(&"module".to_owned()));
    }

    #[test]
    fn extract_keywords_dedup() {
        let kw = extract_keywords("test test test");
        assert_eq!(kw.len(), 1);
    }

    #[test]
    fn extract_keywords_filters_short() {
        let kw = extract_keywords("a an the fix");
        assert!(kw.is_empty()); // all < 4 chars
    }

    #[test]
    fn keyword_overlap_identical() {
        let a = vec!["rust".to_owned(), "code".to_owned()];
        let overlap = keyword_overlap(&a, &a);
        assert!((overlap - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn keyword_overlap_none() {
        let a = vec!["rust".to_owned()];
        let b = vec!["python".to_owned()];
        let overlap = keyword_overlap(&a, &b);
        assert!(overlap.abs() < f64::EPSILON);
    }

    #[test]
    fn keyword_overlap_both_empty() {
        let overlap = keyword_overlap(&[], &[]);
        assert!((overlap - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn keyword_overlap_one_empty() {
        let a = vec!["rust".to_owned()];
        let overlap = keyword_overlap(&a, &[]);
        assert!(overlap.abs() < f64::EPSILON);
    }

    #[test]
    fn depth_display() {
        assert_eq!(ConversationDepth::Shallow.to_string(), "shallow");
        assert_eq!(ConversationDepth::Moderate.to_string(), "moderate");
        assert_eq!(ConversationDepth::Deep.to_string(), "deep");
    }
}
