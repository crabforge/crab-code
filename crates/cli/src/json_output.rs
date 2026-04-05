//! Machine-readable JSON output mode.
//!
//! When `--json` is passed, events are emitted as newline-delimited JSON
//! objects to stdout instead of human-readable text.

use crab_core::event::Event;
use serde_json::{Value, json};

/// Convert an [`Event`] into a JSON line for `--json` mode.
///
/// Returns `None` for events that have no meaningful JSON representation.
pub fn event_to_json(event: &Event) -> Option<Value> {
    match event {
        Event::ContentDelta { index, delta } => Some(json!({
            "type": "content_delta",
            "index": index,
            "delta": delta,
        })),
        Event::ToolUseStart { name, id } => Some(json!({
            "type": "tool_use_start",
            "tool": name,
            "id": id,
        })),
        Event::ToolResult { id, output } => Some(json!({
            "type": "tool_result",
            "id": id,
            "is_error": output.is_error,
            "text": output.text(),
        })),
        Event::Error { message } => Some(json!({
            "type": "error",
            "message": message,
        })),
        Event::TokenWarning {
            usage_pct,
            used,
            limit,
        } => Some(json!({
            "type": "token_warning",
            "usage_pct": usage_pct,
            "used": used,
            "limit": limit,
        })),
        Event::CompactStart { strategy, .. } => Some(json!({
            "type": "compact_start",
            "strategy": strategy,
        })),
        Event::CompactEnd {
            after_tokens,
            removed_messages,
        } => Some(json!({
            "type": "compact_end",
            "after_tokens": after_tokens,
            "removed_messages": removed_messages,
        })),
        _ => None,
    }
}

/// Print a JSON line to stdout (newline-delimited JSON).
pub fn print_json_line(value: &Value) {
    if let Ok(line) = serde_json::to_string(value) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::tool::ToolOutput;

    #[test]
    fn content_delta_to_json() {
        let event = Event::ContentDelta {
            index: 0,
            delta: "hello".into(),
        };
        let json = event_to_json(&event).unwrap();
        assert_eq!(json["type"], "content_delta");
        assert_eq!(json["delta"], "hello");
        assert_eq!(json["index"], 0);
    }

    #[test]
    fn tool_use_start_to_json() {
        let event = Event::ToolUseStart {
            name: "bash".into(),
            id: "t1".into(),
        };
        let json = event_to_json(&event).unwrap();
        assert_eq!(json["type"], "tool_use_start");
        assert_eq!(json["tool"], "bash");
        assert_eq!(json["id"], "t1");
    }

    #[test]
    fn tool_result_success_to_json() {
        let event = Event::ToolResult {
            id: "t1".into(),
            output: ToolOutput::success("done"),
        };
        let json = event_to_json(&event).unwrap();
        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["is_error"], false);
        assert_eq!(json["text"], "done");
    }

    #[test]
    fn tool_result_error_to_json() {
        let event = Event::ToolResult {
            id: "t1".into(),
            output: ToolOutput::error("failed"),
        };
        let json = event_to_json(&event).unwrap();
        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["is_error"], true);
        assert_eq!(json["text"], "failed");
    }

    #[test]
    fn error_event_to_json() {
        let event = Event::Error {
            message: "oh no".into(),
        };
        let json = event_to_json(&event).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["message"], "oh no");
    }

    #[test]
    fn token_warning_to_json() {
        let event = Event::TokenWarning {
            usage_pct: 0.85,
            used: 170_000,
            limit: 200_000,
        };
        let json = event_to_json(&event).unwrap();
        assert_eq!(json["type"], "token_warning");
        // f32 -> JSON: approximate comparison
        let pct = json["usage_pct"].as_f64().unwrap();
        assert!((pct - 0.85).abs() < 0.001);
        assert_eq!(json["used"], 170_000);
        assert_eq!(json["limit"], 200_000);
    }

    #[test]
    fn compact_start_to_json() {
        let event = Event::CompactStart {
            strategy: "sliding-window".into(),
            before_tokens: 180_000,
        };
        let json = event_to_json(&event).unwrap();
        assert_eq!(json["type"], "compact_start");
        assert_eq!(json["strategy"], "sliding-window");
    }

    #[test]
    fn compact_end_to_json() {
        let event = Event::CompactEnd {
            after_tokens: 100_000,
            removed_messages: 5,
        };
        let json = event_to_json(&event).unwrap();
        assert_eq!(json["type"], "compact_end");
        assert_eq!(json["after_tokens"], 100_000);
        assert_eq!(json["removed_messages"], 5);
    }

    #[test]
    fn unknown_event_returns_none() {
        let event = Event::SessionSaved {
            session_id: "s1".into(),
        };
        assert!(event_to_json(&event).is_none());
    }

    #[test]
    fn print_json_line_does_not_panic() {
        let value = json!({"type": "test", "data": 42});
        print_json_line(&value);
    }
}
