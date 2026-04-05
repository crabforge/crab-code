//! Rule-based automatic session summarization.
//!
//! Extracts key actions, files touched, and tools used from a conversation
//! to produce a [`SummaryReport`] without requiring an LLM call.

use std::collections::BTreeSet;

use crab_core::message::{ContentBlock, Message, Role};
use serde::Serialize;

// ── Summary report ──────────────────────────────────────────────────

/// Automatic summary of a conversation session.
#[derive(Debug, Clone, Serialize)]
pub struct SummaryReport {
    /// Short auto-generated title (first user message, truncated).
    pub title: String,
    /// Key actions extracted from tool invocations.
    pub key_actions: Vec<String>,
    /// File paths mentioned or operated on.
    pub files_touched: Vec<String>,
    /// Unique tool names used during the session.
    pub tools_used: Vec<String>,
    /// Outcome classification.
    pub outcome: SessionOutcome,
}

/// High-level outcome of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOutcome {
    /// All tool calls succeeded, assistant provided final answer.
    Success,
    /// Some tool calls failed but the session continued.
    Partial,
    /// No meaningful work was done (very short conversation).
    Minimal,
    /// The session had tool errors and no successful recovery.
    Failed,
}

impl std::fmt::Display for SessionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => f.write_str("success"),
            Self::Partial => f.write_str("partial"),
            Self::Minimal => f.write_str("minimal"),
            Self::Failed => f.write_str("failed"),
        }
    }
}

// ── Summarizer ──────────────────────────────────────────────────────

/// Rule-based session summarizer.
pub struct SessionSummarizer;

impl SessionSummarizer {
    /// Generate a summary from the conversation messages.
    #[must_use]
    pub fn summarize(messages: &[Message]) -> SummaryReport {
        let title = Self::extract_title(messages);
        let key_actions = Self::extract_key_actions(messages);
        let files_touched = Self::extract_files(messages);
        let tools_used = Self::extract_tools(messages);
        let outcome = Self::classify_outcome(messages);

        SummaryReport {
            title,
            key_actions,
            files_touched,
            tools_used,
            outcome,
        }
    }

    /// Title from first user message, truncated to 80 chars.
    fn extract_title(messages: &[Message]) -> String {
        let first_user = messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(Message::text);

        match first_user {
            Some(text) if !text.is_empty() => {
                let trimmed = text.lines().next().unwrap_or(&text);
                if trimmed.len() > 80 {
                    format!("{}...", &trimmed[..77])
                } else {
                    trimmed.to_owned()
                }
            }
            _ => "Untitled session".to_owned(),
        }
    }

    /// Extract key actions from tool use blocks.
    fn extract_key_actions(messages: &[Message]) -> Vec<String> {
        let mut actions = Vec::new();

        for msg in messages {
            for block in &msg.content {
                if let ContentBlock::ToolUse { name, input, .. } = block {
                    let action = Self::describe_tool_action(name, input);
                    actions.push(action);
                }
            }
        }

        actions
    }

    /// Generate a human-readable description of a tool action.
    fn describe_tool_action(name: &str, input: &serde_json::Value) -> String {
        match name {
            "read_file" | "Read" => {
                let path = input
                    .get("file_path")
                    .or_else(|| input.get("path"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                format!("Read file: {path}")
            }
            "write_file" | "Write" => {
                let path = input
                    .get("file_path")
                    .or_else(|| input.get("path"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                format!("Write file: {path}")
            }
            "edit_file" | "Edit" => {
                let path = input
                    .get("file_path")
                    .or_else(|| input.get("path"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                format!("Edit file: {path}")
            }
            "bash" | "Bash" => {
                let cmd = input
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                let short = if cmd.len() > 60 {
                    format!("{}...", &cmd[..57])
                } else {
                    cmd.to_owned()
                };
                format!("Run command: {short}")
            }
            "glob" | "Glob" => {
                let pattern = input
                    .get("pattern")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("*");
                format!("Search files: {pattern}")
            }
            "grep" | "Grep" => {
                let pattern = input
                    .get("pattern")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("...");
                format!("Search content: {pattern}")
            }
            _ => format!("Tool: {name}"),
        }
    }

    /// Extract file paths from tool use inputs.
    fn extract_files(messages: &[Message]) -> Vec<String> {
        let mut files = BTreeSet::new();

        for msg in messages {
            for block in &msg.content {
                if let ContentBlock::ToolUse { input, .. } = block {
                    for key in &["file_path", "path", "old_path", "new_path"] {
                        if let Some(path) = input.get(*key).and_then(serde_json::Value::as_str) {
                            files.insert(path.to_owned());
                        }
                    }
                }
            }
        }

        files.into_iter().collect()
    }

    /// Extract unique tool names used.
    fn extract_tools(messages: &[Message]) -> Vec<String> {
        let mut tools = BTreeSet::new();
        for msg in messages {
            for block in &msg.content {
                if let ContentBlock::ToolUse { name, .. } = block {
                    tools.insert(name.clone());
                }
            }
        }
        tools.into_iter().collect()
    }

    /// Classify the session outcome based on tool results.
    fn classify_outcome(messages: &[Message]) -> SessionOutcome {
        let mut total_results = 0u64;
        let mut successes = 0u64;
        let mut has_assistant_text = false;

        for msg in messages {
            if msg.role == Role::Assistant && !msg.text().is_empty() {
                has_assistant_text = true;
            }
            for block in &msg.content {
                if let ContentBlock::ToolResult { is_error, .. } = block {
                    total_results += 1;
                    if !is_error {
                        successes += 1;
                    }
                }
            }
        }

        if messages.len() < 2 {
            return SessionOutcome::Minimal;
        }

        if total_results == 0 {
            if has_assistant_text {
                SessionOutcome::Success
            } else {
                SessionOutcome::Minimal
            }
        } else if successes == total_results {
            SessionOutcome::Success
        } else if successes == 0 {
            SessionOutcome::Failed
        } else {
            SessionOutcome::Partial
        }
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

    fn tool_use_msg(name: &str, input: serde_json::Value) -> Message {
        Message::new(
            Role::Assistant,
            vec![ContentBlock::tool_use("id1", name, input)],
        )
    }

    fn tool_result_msg(is_error: bool) -> Message {
        Message::tool_result("id1", "result", is_error)
    }

    // ── extract_title ───────────────────────────────────────

    #[test]
    fn title_from_first_user() {
        let msgs = [user_msg("fix the bug in main.rs"), assistant_msg("ok")];
        let title = SessionSummarizer::extract_title(&msgs);
        assert_eq!(title, "fix the bug in main.rs");
    }

    #[test]
    fn title_truncated() {
        let long = "a".repeat(100);
        let msgs = [user_msg(&long)];
        let title = SessionSummarizer::extract_title(&msgs);
        assert_eq!(title.len(), 80);
        assert!(title.ends_with("..."));
    }

    #[test]
    fn title_no_user_messages() {
        let msgs = [assistant_msg("hello")];
        let title = SessionSummarizer::extract_title(&msgs);
        assert_eq!(title, "Untitled session");
    }

    #[test]
    fn title_empty_messages() {
        let title = SessionSummarizer::extract_title(&[]);
        assert_eq!(title, "Untitled session");
    }

    // ── extract_key_actions ─────────────────────────────────

    #[test]
    fn actions_read_file() {
        let msgs = [tool_use_msg(
            "read_file",
            json!({"file_path": "src/main.rs"}),
        )];
        let actions = SessionSummarizer::extract_key_actions(&msgs);
        assert_eq!(actions.len(), 1);
        assert!(actions[0].contains("src/main.rs"));
    }

    #[test]
    fn actions_bash_command() {
        let msgs = [tool_use_msg("bash", json!({"command": "cargo test"}))];
        let actions = SessionSummarizer::extract_key_actions(&msgs);
        assert_eq!(actions[0], "Run command: cargo test");
    }

    #[test]
    fn actions_bash_long_command() {
        let long_cmd = "a".repeat(100);
        let msgs = [tool_use_msg("bash", json!({"command": long_cmd}))];
        let actions = SessionSummarizer::extract_key_actions(&msgs);
        assert!(actions[0].len() < 80);
        assert!(actions[0].ends_with("..."));
    }

    #[test]
    fn actions_unknown_tool() {
        let msgs = [tool_use_msg("custom_tool", json!({}))];
        let actions = SessionSummarizer::extract_key_actions(&msgs);
        assert_eq!(actions[0], "Tool: custom_tool");
    }

    #[test]
    fn actions_glob_tool() {
        let msgs = [tool_use_msg("glob", json!({"pattern": "**/*.rs"}))];
        let actions = SessionSummarizer::extract_key_actions(&msgs);
        assert_eq!(actions[0], "Search files: **/*.rs");
    }

    #[test]
    fn actions_grep_tool() {
        let msgs = [tool_use_msg("grep", json!({"pattern": "fn main"}))];
        let actions = SessionSummarizer::extract_key_actions(&msgs);
        assert_eq!(actions[0], "Search content: fn main");
    }

    // ── extract_files ───────────────────────────────────────

    #[test]
    fn files_from_tool_use() {
        let msgs = [
            tool_use_msg("read_file", json!({"file_path": "src/main.rs"})),
            tool_use_msg("write_file", json!({"file_path": "src/lib.rs"})),
        ];
        let files = SessionSummarizer::extract_files(&msgs);
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"src/main.rs".to_owned()));
        assert!(files.contains(&"src/lib.rs".to_owned()));
    }

    #[test]
    fn files_deduplication() {
        let msgs = [
            tool_use_msg("read_file", json!({"file_path": "a.rs"})),
            tool_use_msg("edit_file", json!({"file_path": "a.rs"})),
        ];
        let files = SessionSummarizer::extract_files(&msgs);
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn files_empty() {
        let files = SessionSummarizer::extract_files(&[user_msg("hi")]);
        assert!(files.is_empty());
    }

    // ── extract_tools ───────────────────────────────────────

    #[test]
    fn tools_unique() {
        let msgs = [
            tool_use_msg("read_file", json!({})),
            tool_use_msg("read_file", json!({})),
            tool_use_msg("bash", json!({})),
        ];
        let tools = SessionSummarizer::extract_tools(&msgs);
        assert_eq!(tools.len(), 2);
    }

    // ── classify_outcome ────────────────────────────────────

    #[test]
    fn outcome_minimal_short() {
        assert_eq!(
            SessionSummarizer::classify_outcome(&[user_msg("hi")]),
            SessionOutcome::Minimal
        );
    }

    #[test]
    fn outcome_success_no_tools() {
        let msgs = [user_msg("hi"), assistant_msg("hello")];
        assert_eq!(
            SessionSummarizer::classify_outcome(&msgs),
            SessionOutcome::Success
        );
    }

    #[test]
    fn outcome_success_all_tools_ok() {
        let msgs = [
            user_msg("do something"),
            tool_use_msg("bash", json!({})),
            tool_result_msg(false),
            assistant_msg("done"),
        ];
        assert_eq!(
            SessionSummarizer::classify_outcome(&msgs),
            SessionOutcome::Success
        );
    }

    #[test]
    fn outcome_partial() {
        let msgs = [
            user_msg("do something"),
            tool_use_msg("bash", json!({})),
            tool_result_msg(false),
            tool_use_msg("bash", json!({})),
            tool_result_msg(true),
            assistant_msg("partially done"),
        ];
        assert_eq!(
            SessionSummarizer::classify_outcome(&msgs),
            SessionOutcome::Partial
        );
    }

    #[test]
    fn outcome_failed() {
        let msgs = [
            user_msg("do something"),
            tool_use_msg("bash", json!({})),
            tool_result_msg(true),
            assistant_msg("failed"),
        ];
        assert_eq!(
            SessionSummarizer::classify_outcome(&msgs),
            SessionOutcome::Failed
        );
    }

    // ── summarize (integration) ─────────────────────────────

    #[test]
    fn summarize_empty() {
        let report = SessionSummarizer::summarize(&[]);
        assert_eq!(report.title, "Untitled session");
        assert!(report.key_actions.is_empty());
        assert!(report.files_touched.is_empty());
        assert!(report.tools_used.is_empty());
        assert_eq!(report.outcome, SessionOutcome::Minimal);
    }

    #[test]
    fn summarize_full_session() {
        let msgs = [
            user_msg("fix the bug in auth module"),
            tool_use_msg("read_file", json!({"file_path": "src/auth.rs"})),
            tool_result_msg(false),
            tool_use_msg("edit_file", json!({"file_path": "src/auth.rs"})),
            tool_result_msg(false),
            tool_use_msg("bash", json!({"command": "cargo test"})),
            tool_result_msg(false),
            assistant_msg("fixed the auth bug and tests pass"),
        ];
        let report = SessionSummarizer::summarize(&msgs);
        assert_eq!(report.title, "fix the bug in auth module");
        assert_eq!(report.key_actions.len(), 3);
        assert_eq!(report.files_touched, vec!["src/auth.rs"]);
        assert_eq!(report.tools_used.len(), 3);
        assert_eq!(report.outcome, SessionOutcome::Success);
    }

    #[test]
    fn summarize_serializes() {
        let report = SessionSummarizer::summarize(&[user_msg("test"), assistant_msg("ok")]);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("title"));
        assert!(json.contains("key_actions"));
        assert!(json.contains("outcome"));
    }

    // ── SessionOutcome display ──────────────────────────────

    #[test]
    fn outcome_display() {
        assert_eq!(SessionOutcome::Success.to_string(), "success");
        assert_eq!(SessionOutcome::Partial.to_string(), "partial");
        assert_eq!(SessionOutcome::Minimal.to_string(), "minimal");
        assert_eq!(SessionOutcome::Failed.to_string(), "failed");
    }

    // ── describe_tool_action variants ───────────────────────

    #[test]
    fn describe_write_tool() {
        let action =
            SessionSummarizer::describe_tool_action("Write", &json!({"file_path": "out.txt"}));
        assert_eq!(action, "Write file: out.txt");
    }

    #[test]
    fn describe_edit_tool() {
        let action =
            SessionSummarizer::describe_tool_action("Edit", &json!({"file_path": "lib.rs"}));
        assert_eq!(action, "Edit file: lib.rs");
    }
}
