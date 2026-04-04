//! Conversion between Chat Completions API types and internal types.

use crab_core::message::{ContentBlock, Message, Role};
use crab_core::model::TokenUsage;

use super::types::{
    ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChatMessage,
    CompletionUsage, ToolCall,
};
use crate::error::Result;
use crate::types::{MessageRequest, MessageResponse, StreamEvent};

/// Convert internal `MessageRequest` to Chat Completions request.
pub fn to_chat_completion_request(req: &MessageRequest<'_>, stream: bool) -> ChatCompletionRequest {
    let mut messages = Vec::new();

    // System prompt → messages[0] with role="system"
    if let Some(system) = &req.system {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(system.clone()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
    }

    // Convert each internal message
    for msg in req.messages.iter() {
        let converted = messages_to_openai(msg);
        messages.extend(converted);
    }

    ChatCompletionRequest {
        model: req.model.0.clone(),
        messages,
        max_tokens: Some(req.max_tokens),
        temperature: req.temperature,
        tools: req.tools.clone(),
        stream,
    }
}

/// Convert an internal `Message` to one or more chat messages.
///
/// A single internal message may produce multiple chat messages because
/// tool results become separate `role="tool"` messages.
fn messages_to_openai(msg: &Message) -> Vec<ChatMessage> {
    let role_str = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
    };

    // Collect tool_use blocks into tool_calls
    let tool_calls: Vec<ToolCall> = msg
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolUse { id, name, input } => Some(ToolCall {
                id: id.clone(),
                call_type: "function".to_string(),
                function: super::types::FunctionCall {
                    name: name.clone(),
                    arguments: input.to_string(),
                },
            }),
            _ => None,
        })
        .collect();

    // Collect text content
    let text: String = msg
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    // Collect tool results as separate messages
    let tool_result_messages: Vec<ChatMessage> = msg
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => Some(ChatMessage {
                role: "tool".to_string(),
                content: Some(content.clone()),
                tool_calls: None,
                tool_call_id: Some(tool_use_id.clone()),
                name: None,
            }),
            _ => None,
        })
        .collect();

    if !tool_result_messages.is_empty() {
        return tool_result_messages;
    }

    let main_message = ChatMessage {
        role: role_str.to_string(),
        content: if text.is_empty() { None } else { Some(text) },
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        tool_call_id: None,
        name: None,
    };

    vec![main_message]
}

/// Convert non-streaming response to internal types.
///
/// # Errors
///
/// Returns `ApiError` if the response has no choices.
pub fn from_chat_completion_response(resp: ChatCompletionResponse) -> Result<MessageResponse> {
    let choice = resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| crate::error::ApiError::Api {
            status: 0,
            message: "no choices in response".to_string(),
        })?;

    let content = chat_message_to_content_blocks(&choice.message);
    let message = Message {
        role: Role::Assistant,
        content,
    };

    let usage = resp
        .usage
        .map(|u| from_completion_usage(&u))
        .unwrap_or_default();

    Ok(MessageResponse {
        id: resp.id,
        message,
        usage,
    })
}

/// Convert a `ChatMessage` to internal `ContentBlock`s.
fn chat_message_to_content_blocks(msg: &ChatMessage) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();

    if let Some(text) = &msg.content
        && !text.is_empty()
    {
        blocks.push(ContentBlock::Text { text: text.clone() });
    }

    if let Some(tool_calls) = &msg.tool_calls {
        for tc in tool_calls {
            let input = serde_json::from_str(&tc.function.arguments)
                .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));
            blocks.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                input,
            });
        }
    }

    blocks
}

/// Convert completion usage to internal `TokenUsage`.
pub const fn from_completion_usage(usage: &CompletionUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
    }
}

/// Convert a streaming chunk to internal `StreamEvent`s.
pub fn chunk_to_stream_event(chunk: &ChatCompletionChunk) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    for choice in &chunk.choices {
        if let Some(content) = &choice.delta.content
            && !content.is_empty()
        {
            events.push(StreamEvent::ContentDelta {
                index: choice.index,
                delta: content.clone(),
            });
        }

        if let Some(tool_calls) = &choice.delta.tool_calls {
            for tc in tool_calls {
                if let Some(func) = &tc.function
                    && let Some(args) = &func.arguments
                    && !args.is_empty()
                {
                    events.push(StreamEvent::ContentDelta {
                        index: tc.index,
                        delta: args.clone(),
                    });
                }
            }
        }

        if choice.finish_reason.is_some() {
            events.push(StreamEvent::MessageStop);
        }
    }

    if let Some(usage) = &chunk.usage {
        events.push(StreamEvent::MessageDelta {
            usage: from_completion_usage(usage),
        });
    }

    events
}
