use std::borrow::Cow;

use crab_api::types::MessageRequest;
use crab_api::LlmBackend;
use crab_core::event::Event;
use crab_core::message::{ContentBlock, Message, Role};
use crab_core::model::ModelId;
use crab_core::tool::{ToolContext, ToolOutput};
use crab_session::Conversation;
use crab_tools::executor::ToolExecutor;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Configuration for the query loop.
pub struct QueryLoopConfig {
    pub model: ModelId,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    /// Tool JSON schemas to send with each API request.
    pub tool_schemas: Vec<serde_json::Value>,
}

/// Core agent loop: user input -> LLM API call -> parse tool calls ->
/// execute tools -> serialize results -> next round.
/// Exits when the model produces a final message without tool calls.
pub async fn query_loop(
    conversation: &mut Conversation,
    backend: &LlmBackend,
    executor: &ToolExecutor,
    tool_ctx: &ToolContext,
    config: &QueryLoopConfig,
    event_tx: mpsc::Sender<Event>,
    cancel: CancellationToken,
) -> crab_common::Result<()> {
    let mut turn_index: usize = 0;

    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }

        // Emit turn start
        let _ = event_tx
            .send(Event::TurnStart { turn_index })
            .await;
        turn_index += 1;

        // Build the API request from conversation state
        let req = MessageRequest {
            model: config.model.clone(),
            messages: Cow::Borrowed(conversation.messages()),
            system: Some(conversation.system_prompt.clone()),
            max_tokens: config.max_tokens,
            tools: config.tool_schemas.clone(),
            temperature: config.temperature,
            cache_breakpoints: vec![],
        };

        // Call the LLM (non-streaming for now; streaming upgrade is a follow-up)
        let response = backend.send_message(req).await.map_err(|e| {
            crab_common::Error::Other(format!("LLM API error: {e}"))
        })?;

        // Record usage
        conversation.record_usage(response.usage.clone());
        let _ = event_tx
            .send(Event::MessageEnd {
                usage: response.usage,
            })
            .await;

        // Add assistant message to conversation
        let assistant_msg = response.message;
        let has_tool_use = assistant_msg.has_tool_use();

        // Emit text content deltas for the TUI
        for (i, block) in assistant_msg.content.iter().enumerate() {
            if let ContentBlock::Text { text } = block {
                let _ = event_tx
                    .send(Event::ContentDelta {
                        index: i,
                        delta: text.clone(),
                    })
                    .await;
            }
        }

        conversation.push(assistant_msg.clone());

        // If no tool use, we're done — model produced a final answer
        if !has_tool_use {
            return Ok(());
        }

        // Extract tool calls and execute them
        let tool_results = execute_tool_calls(
            &assistant_msg,
            executor,
            tool_ctx,
            &event_tx,
            &cancel,
        )
        .await?;

        // Build tool result message and add to conversation
        let result_msg = tool_results_message(tool_results);
        conversation.push(result_msg);
    }
}

/// Execute all tool calls from an assistant message.
///
/// Read-only tools are executed concurrently; write tools sequentially.
async fn execute_tool_calls(
    assistant_msg: &Message,
    executor: &ToolExecutor,
    ctx: &ToolContext,
    event_tx: &mpsc::Sender<Event>,
    cancel: &CancellationToken,
) -> crab_common::Result<Vec<(String, Result<ToolOutput, crab_common::Error>)>> {
    let registry = executor.registry();
    let mut results = Vec::new();

    // Partition into read-only (concurrent) and write (sequential)
    let (reads, writes) = partition_tool_calls(&assistant_msg.content, registry);

    // Execute read-only tools concurrently
    if !reads.is_empty() {
        let read_futures: Vec<_> = reads
            .iter()
            .map(|call| {
                let id = call.id.to_string();
                let name = call.name.to_string();
                let input = call.input.clone();
                let event_tx = event_tx.clone();
                async move {
                    let _ = event_tx
                        .send(Event::ToolUseStart {
                            id: id.clone(),
                            name: name.clone(),
                        })
                        .await;
                    let result = executor.execute(&name, input, ctx).await;
                    let _ = event_tx
                        .send(Event::ToolResult {
                            id: id.clone(),
                            output: match &result {
                                Ok(o) => o.clone(),
                                Err(e) => ToolOutput::error(e.to_string()),
                            },
                        })
                        .await;
                    (id, result)
                }
            })
            .collect();

        let read_results = futures::future::join_all(read_futures).await;
        results.extend(read_results);
    }

    // Execute write tools sequentially
    for call in &writes {
        if cancel.is_cancelled() {
            break;
        }
        let id = call.id.to_string();
        let name = call.name.to_string();

        let _ = event_tx
            .send(Event::ToolUseStart {
                id: id.clone(),
                name: name.clone(),
            })
            .await;

        let result = executor.execute(&name, call.input.clone(), ctx).await;

        let _ = event_tx
            .send(Event::ToolResult {
                id: id.clone(),
                output: match &result {
                    Ok(o) => o.clone(),
                    Err(e) => ToolOutput::error(e.to_string()),
                },
            })
            .await;

        results.push((id, result));
    }

    Ok(results)
}

/// A reference to a tool call within a message.
pub struct ToolCallRef<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub input: &'a serde_json::Value,
}

/// Partition tool calls into read-only (concurrent) and write (sequential) groups.
pub fn partition_tool_calls<'a>(
    blocks: &'a [ContentBlock],
    registry: &crab_tools::registry::ToolRegistry,
) -> (Vec<ToolCallRef<'a>>, Vec<ToolCallRef<'a>>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    for block in blocks {
        if let ContentBlock::ToolUse { id, name, input } = block {
            let call = ToolCallRef { id, name, input };
            let is_read = registry
                .get(name)
                .is_some_and(|t| t.is_read_only());
            if is_read {
                reads.push(call);
            } else {
                writes.push(call);
            }
        }
    }
    (reads, writes)
}

/// Streaming tool executor -- starts tool execution as soon as
/// a `tool_use` block's JSON is fully parsed during SSE streaming.
pub struct StreamingToolExecutor {
    pub pending: Vec<tokio::task::JoinHandle<(String, crab_common::Result<ToolOutput>)>>,
}

impl StreamingToolExecutor {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Spawn a tool execution as soon as its input JSON is complete.
    pub fn spawn(
        &mut self,
        _id: &str,
        name: String,
        input: serde_json::Value,
        ctx: ToolContext,
        tool_fn: impl FnOnce(String, serde_json::Value, ToolContext) -> tokio::task::JoinHandle<(String, crab_common::Result<ToolOutput>)>,
    ) {
        let handle = tool_fn(name, input, ctx);
        self.pending.push(handle);
    }

    /// Collect all pending tool results after `message_stop`.
    pub async fn collect_all(
        &mut self,
    ) -> Vec<(String, crab_common::Result<ToolOutput>)> {
        let mut results = Vec::new();
        for handle in self.pending.drain(..) {
            results.push(handle.await.expect("tool task panicked"));
        }
        results
    }
}

impl Default for StreamingToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a tool result `Message` (role: User) from tool outputs.
pub fn tool_results_message(
    results: Vec<(String, Result<ToolOutput, crab_common::Error>)>,
) -> Message {
    let content: Vec<ContentBlock> = results
        .into_iter()
        .map(|(id, result)| {
            let (text, is_error) = match result {
                Ok(output) => (output.text(), output.is_error),
                Err(e) => (e.to_string(), true),
            };
            ContentBlock::tool_result(id, text, is_error)
        })
        .collect();
    Message::new(Role::User, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::message::ContentBlock;

    #[test]
    fn tool_results_message_builds_user_message() {
        let results = vec![
            ("tu_1".to_string(), Ok(ToolOutput::success("file contents"))),
            (
                "tu_2".to_string(),
                Err(crab_common::Error::Other("not found".into())),
            ),
        ];
        let msg = tool_results_message(results);
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.len(), 2);
        assert!(msg.has_tool_result());

        match &msg.content[0] {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "tu_1");
                assert_eq!(content, "file contents");
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult"),
        }

        match &msg.content[1] {
            ContentBlock::ToolResult {
                tool_use_id,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "tu_2");
                assert!(is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn partition_tool_calls_empty() {
        let registry = crab_tools::registry::ToolRegistry::new();
        let blocks: Vec<ContentBlock> = vec![];
        let (reads, writes) = partition_tool_calls(&blocks, &registry);
        assert!(reads.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn partition_tool_calls_unknown_tools_go_to_writes() {
        let registry = crab_tools::registry::ToolRegistry::new();
        let blocks = vec![ContentBlock::tool_use(
            "tu_1",
            "unknown_tool",
            serde_json::json!({}),
        )];
        let (reads, writes) = partition_tool_calls(&blocks, &registry);
        assert!(reads.is_empty());
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].name, "unknown_tool");
    }

    #[test]
    fn partition_tool_calls_skips_non_tool_blocks() {
        let registry = crab_tools::registry::ToolRegistry::new();
        let blocks = vec![
            ContentBlock::text("some text"),
            ContentBlock::tool_use("tu_1", "bash", serde_json::json!({})),
        ];
        let (reads, writes) = partition_tool_calls(&blocks, &registry);
        assert!(reads.is_empty());
        assert_eq!(writes.len(), 1);
    }

    #[test]
    fn streaming_tool_executor_new_is_empty() {
        let ste = StreamingToolExecutor::new();
        assert!(ste.pending.is_empty());
    }

    #[test]
    fn streaming_tool_executor_default() {
        let ste = StreamingToolExecutor::default();
        assert!(ste.pending.is_empty());
    }

    #[test]
    fn query_loop_config_construction() {
        let config = QueryLoopConfig {
            model: ModelId::from("claude-sonnet-4-20250514"),
            max_tokens: 4096,
            temperature: Some(0.7),
            tool_schemas: vec![],
        };
        assert_eq!(config.model.as_str(), "claude-sonnet-4-20250514");
        assert_eq!(config.max_tokens, 4096);
    }

    #[test]
    fn tool_results_message_single_success() {
        let results = vec![("id1".to_string(), Ok(ToolOutput::success("ok")))];
        let msg = tool_results_message(results);
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.len(), 1);
        match &msg.content[0] {
            ContentBlock::ToolResult {
                is_error, content, ..
            } => {
                assert!(!is_error);
                assert_eq!(content, "ok");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn tool_results_message_single_error() {
        let results = vec![(
            "id1".to_string(),
            Ok(ToolOutput::error("something went wrong")),
        )];
        let msg = tool_results_message(results);
        match &msg.content[0] {
            ContentBlock::ToolResult {
                is_error, content, ..
            } => {
                assert!(is_error);
                assert_eq!(content, "something went wrong");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn tool_results_message_empty() {
        let results: Vec<(String, Result<ToolOutput, crab_common::Error>)> = vec![];
        let msg = tool_results_message(results);
        assert_eq!(msg.role, Role::User);
        assert!(msg.content.is_empty());
    }
}
