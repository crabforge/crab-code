use std::borrow::Cow;

use crab_api::LlmBackend;
use crab_api::capabilities::StreamingUsage;
use crab_api::rate_limit::RetryPolicy;
use crab_api::streaming::StreamingToolParser;
use crab_api::types::{CacheBreakpoint, MessageRequest, StreamEvent};
use crab_core::event::Event;
use crab_core::message::{ContentBlock, Message, Role};
use crab_core::model::{ModelId, TokenUsage};
use crab_core::tool::{ToolContext, ToolOutput};
use crab_session::{CompactionStrategy, ContextAction, ContextManager, Conversation};
use crab_tools::executor::ToolExecutor;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Configuration for the query loop.
#[derive(Clone)]
#[allow(clippy::pub_underscore_fields)]
pub struct QueryLoopConfig {
    pub model: ModelId,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    /// Tool JSON schemas to send with each API request.
    pub tool_schemas: Vec<serde_json::Value>,
    /// Whether to enable prompt caching (Anthropic only).
    pub cache_enabled: bool,
    /// Reserved for future token budget integration.
    pub _token_budget: Option<()>,
    /// Retry policy for API requests. Uses default if `None`.
    pub retry_policy: Option<RetryPolicy>,
}

/// Core agent loop: user input -> LLM SSE stream -> parse tool calls ->
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
    let context_mgr = ContextManager::default();
    let retry_policy = config.retry_policy.clone().unwrap_or_default();

    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }

        // Check context usage and compact if needed
        check_and_compact(conversation, &context_mgr, &event_tx).await;

        // Emit turn start
        let _ = event_tx.send(Event::TurnStart { turn_index }).await;
        turn_index += 1;

        // Build cache breakpoints
        let cache_breakpoints = if config.cache_enabled {
            vec![CacheBreakpoint::System, CacheBreakpoint::Tools]
        } else {
            vec![]
        };

        let max_tokens = config.max_tokens;

        // Build the API request from conversation state
        let req = MessageRequest {
            model: config.model.clone(),
            messages: Cow::Borrowed(conversation.messages()),
            system: Some(conversation.system_prompt.clone()),
            max_tokens,
            tools: config.tool_schemas.clone(),
            temperature: config.temperature,
            cache_breakpoints,
        };

        // Stream the LLM response with retry support
        let (assistant_msg, total_usage, _stop_reason) =
            stream_with_retry(backend, req, &retry_policy, &event_tx, &cancel).await?;

        // Record usage
        conversation.record_usage(total_usage.clone());
        let _ = event_tx
            .send(Event::MessageEnd { usage: total_usage })
            .await;

        // Add assistant message to conversation
        let has_tool_use = assistant_msg.has_tool_use();
        conversation.push(assistant_msg.clone());

        // If no tool use, we're done
        if !has_tool_use {
            return Ok(());
        }

        // Extract tool calls and execute them
        let tool_results =
            execute_tool_calls(&assistant_msg, executor, tool_ctx, &event_tx, &cancel).await?;

        // Build tool result message and add to conversation
        let result_msg = tool_results_message(tool_results);
        conversation.push(result_msg);
    }
}

/// Retry wrapper around `stream_response`. Retries on transient errors
/// (connection, timeout, rate limit) using the provided `RetryPolicy`.
async fn stream_with_retry(
    backend: &LlmBackend,
    req: MessageRequest<'_>,
    policy: &RetryPolicy,
    event_tx: &mpsc::Sender<Event>,
    cancel: &CancellationToken,
) -> crab_common::Result<(Message, TokenUsage, Option<String>)> {
    let mut attempt = 0u32;
    loop {
        let req_clone = req.clone();
        match stream_response(backend, req_clone, event_tx, cancel).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                // Check if we should retry: only retry transient errors
                // and only if we haven't exceeded the retry limit
                let is_transient = is_transient_error(&e);
                if is_transient && attempt < policy.max_retries {
                    let delay = policy.delay_for_attempt(attempt);
                    let _ = event_tx
                        .send(Event::Error {
                            message: format!(
                                "Retrying after error (attempt {}/{}): {e}",
                                attempt + 1,
                                policy.max_retries
                            ),
                        })
                        .await;
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                } else {
                    return Err(e);
                }
            }
        }
    }
}

/// Check if a `crab_common::Error` represents a transient/retryable condition.
fn is_transient_error(err: &crab_common::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("timeout")
        || msg.contains("timed out")
        || msg.contains("connection")
        || msg.contains("rate limit")
        || msg.contains("429")
        || msg.contains("529")
        || msg.contains("overloaded")
}

/// Stream an LLM response, assembling the assistant message from SSE events.
///
/// Uses `StreamingToolParser` for incremental `tool_use` block parsing and
/// `StreamingUsage` for accurate token accumulation.
///
/// Returns the assembled message, total usage, and stop reason.
async fn stream_response(
    backend: &LlmBackend,
    req: MessageRequest<'_>,
    event_tx: &mpsc::Sender<Event>,
    cancel: &CancellationToken,
) -> crab_common::Result<(Message, TokenUsage, Option<String>)> {
    let mut stream = std::pin::pin!(backend.stream_message(req));

    // Use StreamingToolParser for incremental tool_use parsing
    let mut tool_parser = StreamingToolParser::new();
    // Use StreamingUsage for accurate token accumulation
    let mut usage_tracker = StreamingUsage::new();

    while let Some(event) = stream.next().await {
        if cancel.is_cancelled() {
            break;
        }

        let event =
            event.map_err(|e| crab_common::Error::Other(format!("SSE stream error: {e}")))?;

        // Update usage tracker
        usage_tracker.update(&event);

        // Feed event to tool parser for incremental tool_use assembly
        tool_parser.process(&event);

        match &event {
            StreamEvent::MessageStart { id, .. } => {
                let _ = event_tx.send(Event::MessageStart { id: id.clone() }).await;
            }
            StreamEvent::ContentDelta { index, delta } => {
                let _ = event_tx
                    .send(Event::ContentDelta {
                        index: *index,
                        delta: delta.clone(),
                    })
                    .await;
            }
            StreamEvent::ContentBlockStop { index } => {
                let _ = event_tx
                    .send(Event::ContentBlockStop { index: *index })
                    .await;
            }
            StreamEvent::Error { message } => {
                let _ = event_tx
                    .send(Event::Error {
                        message: message.clone(),
                    })
                    .await;
                return Err(crab_common::Error::Other(format!(
                    "SSE stream error: {message}"
                )));
            }
            StreamEvent::ContentBlockStart { .. }
            | StreamEvent::MessageDelta { .. }
            | StreamEvent::MessageStop => {}
        }
    }

    // Extract stop reason before consuming usage_tracker
    let stop_reason = usage_tracker.stop_reason().map(String::from);

    // Assemble content blocks into a Message using the tool parser
    let mut content: Vec<ContentBlock> = Vec::new();

    // Add text content if any
    let text = tool_parser.text();
    if !text.is_empty() {
        content.push(ContentBlock::text(text));
    }

    // Add completed tool_use blocks from the streaming parser
    for acc in tool_parser.completed_tools() {
        content.push(ContentBlock::ToolUse {
            id: acc.id.clone(),
            name: acc.name.clone(),
            input: acc.parse_input(),
        });
    }

    // Add any in-progress tools that didn't get a ContentBlockStop
    for acc in tool_parser.in_progress_tools() {
        if let Some(input) = acc.try_parse_input() {
            content.push(ContentBlock::ToolUse {
                id: acc.id.clone(),
                name: acc.name.clone(),
                input,
            });
        }
    }

    let message = Message::new(Role::Assistant, content);
    let total_usage = usage_tracker.into_usage();

    Ok((message, total_usage, stop_reason))
}

/// Check context usage and compact if needed.
#[allow(clippy::cast_precision_loss)]
async fn check_and_compact(
    conversation: &mut Conversation,
    context_mgr: &ContextManager,
    event_tx: &mpsc::Sender<Event>,
) {
    match context_mgr.check(conversation) {
        ContextAction::NeedsCompaction {
            used,
            limit,
            percent,
        } => {
            if let Some(strategy) = CompactionStrategy::for_usage(percent) {
                let before_tokens = conversation.estimated_tokens();
                let strategy_name = format!("{strategy:?}");
                let _ = event_tx
                    .send(Event::CompactStart {
                        strategy: strategy_name,
                        before_tokens,
                    })
                    .await;

                // Use truncation directly (LLM-based compaction needs CompactionClient)
                let budget = limit * 60 / 100;
                let removed = conversation.inner.truncate_to_budget(budget);

                let _ = event_tx
                    .send(Event::CompactEnd {
                        after_tokens: conversation.estimated_tokens(),
                        removed_messages: removed,
                    })
                    .await;
            } else {
                let _ = event_tx
                    .send(Event::TokenWarning {
                        usage_pct: used as f32 / limit as f32,
                        used,
                        limit,
                    })
                    .await;
            }
        }
        ContextAction::Warning { used, limit, .. } => {
            let _ = event_tx
                .send(Event::TokenWarning {
                    usage_pct: used as f32 / limit as f32,
                    used,
                    limit,
                })
                .await;
        }
        ContextAction::Ok => {}
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
            let is_read = registry.get(name).is_some_and(|t| t.is_read_only());
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
        tool_fn: impl FnOnce(
            String,
            serde_json::Value,
            ToolContext,
        )
            -> tokio::task::JoinHandle<(String, crab_common::Result<ToolOutput>)>,
    ) {
        let handle = tool_fn(name, input, ctx);
        self.pending.push(handle);
    }

    /// Collect all pending tool results after `message_stop`.
    pub async fn collect_all(&mut self) -> Vec<(String, crab_common::Result<ToolOutput>)> {
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
            cache_enabled: false,
            _token_budget: None,
            retry_policy: None,
        };
        assert_eq!(config.model.as_str(), "claude-sonnet-4-20250514");
        assert_eq!(config.max_tokens, 4096);
    }

    #[test]
    fn query_loop_config_with_retry_policy() {
        let policy = RetryPolicy::aggressive();
        let config = QueryLoopConfig {
            model: ModelId::from("claude-sonnet-4-20250514"),
            max_tokens: 4096,
            temperature: None,
            tool_schemas: vec![],
            cache_enabled: false,
            _token_budget: None,
            retry_policy: Some(policy),
        };
        assert!(config.retry_policy.is_some());
        assert_eq!(config.retry_policy.unwrap().max_retries, 5);
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

    #[test]
    fn transient_error_timeout() {
        let err = crab_common::Error::Other("request timed out".into());
        assert!(is_transient_error(&err));
    }

    #[test]
    fn transient_error_connection() {
        let err = crab_common::Error::Other("connection refused".into());
        assert!(is_transient_error(&err));
    }

    #[test]
    fn transient_error_rate_limit() {
        let err = crab_common::Error::Other("SSE stream error: rate limit exceeded 429".into());
        assert!(is_transient_error(&err));
    }

    #[test]
    fn transient_error_overloaded() {
        let err = crab_common::Error::Other("server overloaded".into());
        assert!(is_transient_error(&err));
    }

    #[test]
    fn non_transient_error_json() {
        let err = crab_common::Error::Other("invalid JSON".into());
        assert!(!is_transient_error(&err));
    }

    #[test]
    fn non_transient_error_auth() {
        let err = crab_common::Error::Other("unauthorized: invalid API key".into());
        assert!(!is_transient_error(&err));
    }
}
