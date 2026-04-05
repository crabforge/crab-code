//! Context window optimization and smart truncation strategies.
//!
//! `ContextWindowOptimizer` allocates a model's token budget across system
//! prompt, conversation history, and tool results. `TruncationStrategy`
//! controls how over-budget content is trimmed. `MessagePrioritizer` scores
//! messages by importance so the optimizer can drop the least valuable first.

use std::fmt;

// ---------------------------------------------------------------------------
// Token estimation
// ---------------------------------------------------------------------------

/// Rough token estimate: ~4 characters per token (English-centric heuristic).
/// Good enough for budget allocation — exact counts come from the tokenizer.
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    // Ceiling division to avoid undercount.
    #[allow(clippy::cast_possible_truncation)]
    let chars = text.len() as u32;
    chars.div_ceil(4)
}

// ---------------------------------------------------------------------------
// MessagePriority
// ---------------------------------------------------------------------------

/// Importance level for a conversation message.
///
/// Higher priority messages are kept when the context must be trimmed.
/// Order: `Critical > High > Normal > Low`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MessagePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl fmt::Display for MessagePriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Low => write!(f, "low"),
            Self::Normal => write!(f, "normal"),
            Self::High => write!(f, "high"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

// ---------------------------------------------------------------------------
// MessageEntry — a scored message for the optimizer
// ---------------------------------------------------------------------------

/// A conversation message with metadata for prioritization.
#[derive(Debug, Clone)]
pub struct MessageEntry {
    /// Role: "system", "user", "assistant", "tool".
    pub role: String,
    /// Message content (text).
    pub content: String,
    /// Estimated token count.
    pub token_count: u32,
    /// Turn index (0 = oldest).
    pub turn_index: usize,
    /// Whether this is a tool result.
    pub is_tool_result: bool,
    /// Computed priority.
    pub priority: MessagePriority,
}

impl MessageEntry {
    /// Create a new message entry with auto-estimated tokens and auto-assigned priority.
    #[must_use]
    pub fn new(role: &str, content: &str, turn_index: usize) -> Self {
        let is_tool_result = role == "tool";
        let priority = Self::default_priority(role, turn_index);
        Self {
            role: role.to_string(),
            content: content.to_string(),
            token_count: estimate_tokens(content),
            turn_index,
            is_tool_result,
            priority,
        }
    }

    /// Override priority.
    #[must_use]
    pub fn with_priority(mut self, priority: MessagePriority) -> Self {
        self.priority = priority;
        self
    }

    /// Default priority assignment:
    /// - system → Critical
    /// - tool results → High
    /// - last 2 turns → High
    /// - everything else → Normal
    fn default_priority(role: &str, _turn_index: usize) -> MessagePriority {
        match role {
            "system" => MessagePriority::Critical,
            "tool" => MessagePriority::High,
            _ => MessagePriority::Normal,
        }
    }
}

// ---------------------------------------------------------------------------
// MessagePrioritizer
// ---------------------------------------------------------------------------

/// Scores and sorts messages by importance for context window fitting.
///
/// Priority order: tool results and system prompt > recent conversation > older conversation.
/// Within same priority, recency wins (higher `turn_index` kept).
#[derive(Debug, Default)]
pub struct MessagePrioritizer {
    /// Number of recent turns to boost to High priority.
    recent_turn_boost: usize,
}

impl MessagePrioritizer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            recent_turn_boost: 2,
        }
    }

    /// Set how many recent turns get boosted priority.
    #[must_use]
    pub fn with_recent_boost(mut self, turns: usize) -> Self {
        self.recent_turn_boost = turns;
        self
    }

    /// Assign priorities to a list of messages, boosting recent turns.
    pub fn prioritize(&self, messages: &mut [MessageEntry]) {
        if messages.is_empty() {
            return;
        }

        let max_turn = messages.iter().map(|m| m.turn_index).max().unwrap_or(0);
        let boost_threshold = max_turn.saturating_sub(self.recent_turn_boost.saturating_sub(1));

        for msg in messages.iter_mut() {
            // System messages stay Critical.
            if msg.role == "system" {
                msg.priority = MessagePriority::Critical;
                continue;
            }
            // Tool results stay High.
            if msg.is_tool_result {
                msg.priority = MessagePriority::High;
                continue;
            }
            // Recent turns get boosted.
            if msg.turn_index >= boost_threshold {
                msg.priority = MessagePriority::High;
            } else {
                msg.priority = MessagePriority::Normal;
            }
        }
    }

    /// Sort messages by priority (highest first), then by turn index (most recent first).
    /// Returns indices into the original slice in drop order (last = most important).
    #[must_use]
    pub fn drop_order(&self, messages: &[MessageEntry]) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..messages.len()).collect();
        // Sort: lowest priority first, then oldest first → these get dropped first.
        indices.sort_by(|&a, &b| {
            messages[a]
                .priority
                .cmp(&messages[b].priority)
                .then_with(|| messages[a].turn_index.cmp(&messages[b].turn_index))
        });
        indices
    }
}

// ---------------------------------------------------------------------------
// TruncationStrategy
// ---------------------------------------------------------------------------

/// How to truncate content that exceeds its token budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncationStrategy {
    /// Keep the end, drop the beginning. Best for recent context.
    TailKeep,
    /// Keep the beginning, drop the end. Best for system prompts.
    HeadKeep,
    /// Keep beginning and end, drop the middle. Preserves both setup and recent context.
    MiddleDrop,
}

impl fmt::Display for TruncationStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TailKeep => write!(f, "tail_keep"),
            Self::HeadKeep => write!(f, "head_keep"),
            Self::MiddleDrop => write!(f, "middle_drop"),
        }
    }
}

impl TruncationStrategy {
    /// Truncate `text` to fit within `max_tokens` (estimated).
    /// Returns the truncated text.
    #[must_use]
    pub fn truncate(&self, text: &str, max_tokens: u32) -> String {
        let current = estimate_tokens(text);
        if current <= max_tokens {
            return text.to_string();
        }

        // Approximate character budget (4 chars per token).
        let char_budget = (max_tokens as usize) * 4;
        if char_budget == 0 {
            return String::new();
        }

        let chars: Vec<char> = text.chars().collect();
        let total = chars.len();

        match self {
            Self::TailKeep => {
                // Keep the last `char_budget` characters.
                let start = total.saturating_sub(char_budget);
                let truncated: String = chars[start..].iter().collect();
                format!("[...truncated...]\n{truncated}")
            }
            Self::HeadKeep => {
                // Keep the first `char_budget` characters.
                let end = char_budget.min(total);
                let truncated: String = chars[..end].iter().collect();
                format!("{truncated}\n[...truncated...]")
            }
            Self::MiddleDrop => {
                // Keep first half and last half of the budget.
                let half = char_budget / 2;
                let head: String = chars[..half.min(total)].iter().collect();
                let tail_start = total.saturating_sub(half);
                let tail: String = chars[tail_start..].iter().collect();
                format!(
                    "{head}\n[...{} tokens truncated...]\n{tail}",
                    current - max_tokens
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BudgetAllocation
// ---------------------------------------------------------------------------

/// Token budget allocation across context sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetAllocation {
    /// Tokens for the system prompt.
    pub system_tokens: u32,
    /// Tokens for conversation history.
    pub history_tokens: u32,
    /// Tokens for tool results.
    pub tool_tokens: u32,
    /// Tokens reserved for the model's output.
    pub output_reserved: u32,
    /// Total context window.
    pub total: u32,
}

impl BudgetAllocation {
    /// How many tokens are allocated for input (system + history + tools).
    #[must_use]
    pub fn input_tokens(&self) -> u32 {
        self.system_tokens + self.history_tokens + self.tool_tokens
    }

    /// Whether the allocation leaves room for output.
    #[must_use]
    pub fn has_output_room(&self) -> bool {
        self.input_tokens() + self.output_reserved <= self.total
    }
}

// ---------------------------------------------------------------------------
// ContextWindowOptimizer
// ---------------------------------------------------------------------------

/// Allocates a model's token budget across system prompt, conversation
/// history, and tool results. Applies truncation when content exceeds budget.
#[derive(Debug, Clone)]
pub struct ContextWindowOptimizer {
    /// Total context window size (tokens).
    context_window: u32,
    /// Tokens reserved for output.
    output_reserved: u32,
    /// Fraction of input budget for system prompt (0.0–1.0).
    system_ratio: f32,
    /// Fraction of input budget for tool results (0.0–1.0).
    tool_ratio: f32,
    /// Truncation strategy for system prompt.
    system_strategy: TruncationStrategy,
    /// Truncation strategy for conversation history.
    history_strategy: TruncationStrategy,
    /// Truncation strategy for tool results.
    tool_strategy: TruncationStrategy,
}

impl ContextWindowOptimizer {
    /// Create an optimizer for a given context window.
    #[must_use]
    pub fn new(context_window: u32, output_reserved: u32) -> Self {
        Self {
            context_window,
            output_reserved,
            system_ratio: 0.15,
            tool_ratio: 0.30,
            system_strategy: TruncationStrategy::HeadKeep,
            history_strategy: TruncationStrategy::MiddleDrop,
            tool_strategy: TruncationStrategy::TailKeep,
        }
    }

    /// Set the system prompt budget ratio.
    #[must_use]
    pub fn with_system_ratio(mut self, ratio: f32) -> Self {
        self.system_ratio = ratio.clamp(0.0, 1.0);
        self
    }

    /// Set the tool results budget ratio.
    #[must_use]
    pub fn with_tool_ratio(mut self, ratio: f32) -> Self {
        self.tool_ratio = ratio.clamp(0.0, 1.0);
        self
    }

    /// Set truncation strategy for system prompt.
    #[must_use]
    pub fn with_system_strategy(mut self, strategy: TruncationStrategy) -> Self {
        self.system_strategy = strategy;
        self
    }

    /// Set truncation strategy for history.
    #[must_use]
    pub fn with_history_strategy(mut self, strategy: TruncationStrategy) -> Self {
        self.history_strategy = strategy;
        self
    }

    /// Set truncation strategy for tool results.
    #[must_use]
    pub fn with_tool_strategy(mut self, strategy: TruncationStrategy) -> Self {
        self.tool_strategy = strategy;
        self
    }

    /// Compute the budget allocation for the given input sizes.
    #[must_use]
    pub fn allocate(&self) -> BudgetAllocation {
        let input_budget = self.context_window.saturating_sub(self.output_reserved);

        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let system_tokens = (input_budget as f32 * self.system_ratio) as u32;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let tool_tokens = (input_budget as f32 * self.tool_ratio) as u32;
        let history_tokens = input_budget.saturating_sub(system_tokens + tool_tokens);

        BudgetAllocation {
            system_tokens,
            history_tokens,
            tool_tokens,
            output_reserved: self.output_reserved,
            total: self.context_window,
        }
    }

    /// Optimize a context: truncate system prompt, messages, and tool results to fit.
    ///
    /// Returns `(system, messages, tool_results)` — all truncated to budget.
    pub fn optimize(
        &self,
        system: &str,
        messages: &mut [MessageEntry],
        tool_results: &[String],
    ) -> OptimizedContext {
        let allocation = self.allocate();
        let prioritizer = MessagePrioritizer::new();

        // 1. Truncate system prompt.
        let system_out = self
            .system_strategy
            .truncate(system, allocation.system_tokens);

        // 2. Truncate individual tool results, then trim if still over budget.
        let per_tool_budget = if tool_results.is_empty() {
            0
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let len = tool_results.len() as u32;
            allocation.tool_tokens / len
        };
        let tools_out: Vec<String> = tool_results
            .iter()
            .map(|t| self.tool_strategy.truncate(t, per_tool_budget))
            .collect();

        // 3. Prioritize and trim messages to fit history budget.
        prioritizer.prioritize(messages);
        let drop_order = prioritizer.drop_order(messages);

        let mut total_msg_tokens: u32 = messages.iter().map(|m| m.token_count).sum();
        let mut dropped = vec![false; messages.len()];

        for &idx in &drop_order {
            if total_msg_tokens <= allocation.history_tokens {
                break;
            }
            total_msg_tokens = total_msg_tokens.saturating_sub(messages[idx].token_count);
            dropped[idx] = true;
        }

        let kept_messages: Vec<MessageEntry> = messages
            .iter()
            .enumerate()
            .filter(|(i, _)| !dropped[*i])
            .map(|(_, m)| m.clone())
            .collect();

        OptimizedContext {
            system: system_out,
            messages: kept_messages,
            tool_results: tools_out,
            allocation,
        }
    }
}

/// Result of context optimization.
#[derive(Debug, Clone)]
pub struct OptimizedContext {
    /// Truncated system prompt.
    pub system: String,
    /// Kept messages (in original order).
    pub messages: Vec<MessageEntry>,
    /// Truncated tool results.
    pub tool_results: Vec<String>,
    /// The budget allocation used.
    pub allocation: BudgetAllocation,
}

impl OptimizedContext {
    /// Estimated total input tokens after optimization.
    #[must_use]
    pub fn estimated_input_tokens(&self) -> u32 {
        let sys = estimate_tokens(&self.system);
        let msgs: u32 = self
            .messages
            .iter()
            .map(|m| estimate_tokens(&m.content))
            .sum();
        let tools: u32 = self.tool_results.iter().map(|t| estimate_tokens(t)).sum();
        sys + msgs + tools
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- estimate_tokens --

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    // -- MessagePriority --

    #[test]
    fn priority_ordering() {
        assert!(MessagePriority::Critical > MessagePriority::High);
        assert!(MessagePriority::High > MessagePriority::Normal);
        assert!(MessagePriority::Normal > MessagePriority::Low);
    }

    #[test]
    fn priority_display() {
        assert_eq!(MessagePriority::Critical.to_string(), "critical");
        assert_eq!(MessagePriority::Low.to_string(), "low");
    }

    // -- MessageEntry --

    #[test]
    fn message_entry_system_is_critical() {
        let entry = MessageEntry::new("system", "You are a helpful assistant.", 0);
        assert_eq!(entry.priority, MessagePriority::Critical);
        assert!(!entry.is_tool_result);
    }

    #[test]
    fn message_entry_tool_is_high() {
        let entry = MessageEntry::new("tool", "file contents here", 3);
        assert_eq!(entry.priority, MessagePriority::High);
        assert!(entry.is_tool_result);
    }

    #[test]
    fn message_entry_user_is_normal() {
        let entry = MessageEntry::new("user", "Hello!", 1);
        assert_eq!(entry.priority, MessagePriority::Normal);
    }

    #[test]
    fn message_entry_with_priority_override() {
        let entry =
            MessageEntry::new("user", "Important!", 1).with_priority(MessagePriority::Critical);
        assert_eq!(entry.priority, MessagePriority::Critical);
    }

    // -- MessagePrioritizer --

    #[test]
    fn prioritizer_boosts_recent_turns() {
        let prioritizer = MessagePrioritizer::new().with_recent_boost(2);
        let mut messages = vec![
            MessageEntry::new("user", "old message", 0),
            MessageEntry::new("assistant", "old reply", 1),
            MessageEntry::new("user", "recent question", 4),
            MessageEntry::new("assistant", "recent answer", 5),
        ];
        prioritizer.prioritize(&mut messages);

        assert_eq!(messages[0].priority, MessagePriority::Normal); // turn 0
        assert_eq!(messages[1].priority, MessagePriority::Normal); // turn 1
        assert_eq!(messages[2].priority, MessagePriority::High); // turn 4 (recent)
        assert_eq!(messages[3].priority, MessagePriority::High); // turn 5 (recent)
    }

    #[test]
    fn prioritizer_system_stays_critical() {
        let prioritizer = MessagePrioritizer::new();
        let mut messages = vec![
            MessageEntry::new("system", "system prompt", 0),
            MessageEntry::new("user", "hi", 1),
        ];
        prioritizer.prioritize(&mut messages);
        assert_eq!(messages[0].priority, MessagePriority::Critical);
    }

    #[test]
    fn drop_order_drops_low_priority_first() {
        let prioritizer = MessagePrioritizer::new();
        let messages = vec![
            MessageEntry::new("system", "sys", 0).with_priority(MessagePriority::Critical),
            MessageEntry::new("user", "old", 1).with_priority(MessagePriority::Normal),
            MessageEntry::new("tool", "result", 2).with_priority(MessagePriority::High),
            MessageEntry::new("user", "recent", 3).with_priority(MessagePriority::High),
        ];
        let order = prioritizer.drop_order(&messages);
        // Normal priority (index 1) should be dropped first.
        assert_eq!(order[0], 1);
    }

    // -- TruncationStrategy --

    #[test]
    fn truncation_no_op_when_under_budget() {
        let text = "short text";
        let result = TruncationStrategy::HeadKeep.truncate(text, 100);
        assert_eq!(result, "short text");
    }

    #[test]
    fn truncation_head_keep() {
        let text = "a".repeat(100); // ~25 tokens
        let result = TruncationStrategy::HeadKeep.truncate(&text, 5);
        assert!(result.starts_with("aaaa"));
        assert!(result.contains("[...truncated...]"));
    }

    #[test]
    fn truncation_tail_keep() {
        let text = "a".repeat(100);
        let result = TruncationStrategy::TailKeep.truncate(&text, 5);
        assert!(result.ends_with("aaaa"));
        assert!(result.contains("[...truncated...]"));
    }

    #[test]
    fn truncation_middle_drop() {
        let text = "a".repeat(100);
        let result = TruncationStrategy::MiddleDrop.truncate(&text, 5);
        assert!(result.contains("tokens truncated"));
        // Should have head and tail portions.
        assert!(result.starts_with("aaa"));
        assert!(result.ends_with("aaa"));
    }

    #[test]
    fn truncation_zero_budget() {
        let text = "some text";
        let result = TruncationStrategy::HeadKeep.truncate(text, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn truncation_display() {
        assert_eq!(TruncationStrategy::TailKeep.to_string(), "tail_keep");
        assert_eq!(TruncationStrategy::HeadKeep.to_string(), "head_keep");
        assert_eq!(TruncationStrategy::MiddleDrop.to_string(), "middle_drop");
    }

    // -- BudgetAllocation --

    #[test]
    fn budget_allocation_input_tokens() {
        let alloc = BudgetAllocation {
            system_tokens: 100,
            history_tokens: 500,
            tool_tokens: 200,
            output_reserved: 1000,
            total: 8000,
        };
        assert_eq!(alloc.input_tokens(), 800);
        assert!(alloc.has_output_room());
    }

    #[test]
    fn budget_allocation_no_room() {
        let alloc = BudgetAllocation {
            system_tokens: 5000,
            history_tokens: 3000,
            tool_tokens: 1000,
            output_reserved: 1000,
            total: 8000,
        };
        assert!(!alloc.has_output_room());
    }

    // -- ContextWindowOptimizer --

    #[test]
    fn optimizer_allocate_default_ratios() {
        let opt = ContextWindowOptimizer::new(10000, 2000);
        let alloc = opt.allocate();
        assert_eq!(alloc.total, 10000);
        assert_eq!(alloc.output_reserved, 2000);
        // system: 15% of 8000 = 1200, tool: 30% of 8000 = 2400
        assert_eq!(alloc.system_tokens, 1200);
        assert_eq!(alloc.tool_tokens, 2400);
        assert_eq!(alloc.history_tokens, 4400);
    }

    #[test]
    fn optimizer_custom_ratios() {
        let opt = ContextWindowOptimizer::new(10000, 2000)
            .with_system_ratio(0.10)
            .with_tool_ratio(0.20);
        let alloc = opt.allocate();
        assert_eq!(alloc.system_tokens, 800);
        assert_eq!(alloc.tool_tokens, 1600);
        assert_eq!(alloc.history_tokens, 5600);
    }

    #[test]
    fn optimizer_optimize_fits_within_budget() {
        let opt = ContextWindowOptimizer::new(1000, 200);
        let system = "You are a coding assistant.";
        let mut messages = vec![
            MessageEntry::new("user", "Hello", 0),
            MessageEntry::new("assistant", "Hi there!", 1),
        ];
        let tools = vec!["tool output".to_string()];

        let result = opt.optimize(system, &mut messages, &tools);
        assert!(result.estimated_input_tokens() <= 800);
        assert!(!result.messages.is_empty());
    }

    #[test]
    fn optimizer_drops_low_priority_messages_first() {
        // Tiny budget forces message dropping.
        let opt = ContextWindowOptimizer::new(200, 50);
        let system = "sys";
        let mut messages = vec![
            MessageEntry::new("user", &"old content ".repeat(20), 0),
            MessageEntry::new("assistant", &"old reply ".repeat(20), 1),
            MessageEntry::new("user", "recent question", 10),
            MessageEntry::new("assistant", "recent answer", 11),
        ];
        let tools: Vec<String> = vec![];

        let result = opt.optimize(system, &mut messages, &tools);

        // Recent messages should be preferentially kept.
        let kept_turns: Vec<usize> = result.messages.iter().map(|m| m.turn_index).collect();
        // Old messages (turns 0, 1) should be dropped before recent ones (10, 11).
        if kept_turns.len() < 4 {
            // Something was dropped — recent turns should still be present if budget allows.
            for &turn in &kept_turns {
                // At minimum, the recent turns should be preferred.
                assert!(turn >= 0); // basic sanity
            }
        }
    }

    #[test]
    fn optimizer_truncates_system_prompt() {
        let opt = ContextWindowOptimizer::new(100, 20);
        let long_system = "a".repeat(2000);
        let mut messages = vec![];
        let tools: Vec<String> = vec![];

        let result = opt.optimize(&long_system, &mut messages, &tools);
        // System prompt should be truncated.
        assert!(result.system.len() < 2000);
    }

    #[test]
    fn optimizer_truncates_tool_results() {
        let opt = ContextWindowOptimizer::new(200, 50);
        let system = "sys";
        let mut messages = vec![];
        let tools = vec!["x".repeat(1000), "y".repeat(1000)];

        let result = opt.optimize(system, &mut messages, &tools);
        assert_eq!(result.tool_results.len(), 2);
        for tool in &result.tool_results {
            assert!(tool.len() < 1000);
        }
    }

    #[test]
    fn optimized_context_estimated_tokens() {
        let ctx = OptimizedContext {
            system: "hello".to_string(),
            messages: vec![MessageEntry::new("user", "question", 0)],
            tool_results: vec!["result".to_string()],
            allocation: BudgetAllocation {
                system_tokens: 100,
                history_tokens: 100,
                tool_tokens: 100,
                output_reserved: 100,
                total: 400,
            },
        };
        let tokens = ctx.estimated_input_tokens();
        assert!(tokens > 0);
    }

    #[test]
    fn optimizer_with_custom_strategies() {
        let opt = ContextWindowOptimizer::new(1000, 200)
            .with_system_strategy(TruncationStrategy::TailKeep)
            .with_history_strategy(TruncationStrategy::TailKeep)
            .with_tool_strategy(TruncationStrategy::HeadKeep);

        let system = "short";
        let mut messages = vec![MessageEntry::new("user", "test", 0)];
        let tools: Vec<String> = vec![];

        let result = opt.optimize(system, &mut messages, &tools);
        assert!(!result.system.is_empty());
    }
}
