//! Protocol compliance tests for the MCP server.
//!
//! These tests verify JSON-RPC error codes, capability negotiation,
//! response ID matching, and concurrent request handling per the MCP spec.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use crate::protocol::{
        InitializeResult, JsonRpcRequest, McpPrompt, McpResource, McpToolDef, PromptGetResult,
        ResourceContent, ResourceReadResult, ToolCallResult, ToolResultContent, method,
    };
    use crate::server::{McpServer, PromptHandler, ResourceHandler, ToolHandler};

    // ─── Test fixtures ───

    struct ComplianceHandler;

    impl ToolHandler for ComplianceHandler {
        fn list_tools(&self) -> Vec<McpToolDef> {
            vec![McpToolDef {
                name: "test_tool".into(),
                description: "A test tool".into(),
                input_schema: json!({"type": "object"}),
            }]
        }

        fn call_tool(
            &self,
            _name: &str,
            _arguments: serde_json::Value,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
        {
            Box::pin(async move {
                ToolCallResult {
                    content: vec![ToolResultContent::Text { text: "ok".into() }],
                    is_error: false,
                }
            })
        }
    }

    struct ComplianceResourceHandler;

    impl ResourceHandler for ComplianceResourceHandler {
        fn list_resources(&self) -> Vec<McpResource> {
            vec![McpResource {
                uri: "file:///test.txt".into(),
                name: "test.txt".into(),
                description: None,
                mime_type: Some("text/plain".into()),
            }]
        }

        fn read_resource(
            &self,
            uri: &str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<ResourceReadResult, String>> + Send + '_>,
        > {
            let uri = uri.to_string();
            Box::pin(async move {
                Ok(ResourceReadResult {
                    contents: vec![ResourceContent {
                        uri,
                        mime_type: Some("text/plain".into()),
                        text: Some("content".into()),
                    }],
                })
            })
        }
    }

    struct CompliancePromptHandler;

    impl PromptHandler for CompliancePromptHandler {
        fn list_prompts(&self) -> Vec<McpPrompt> {
            vec![McpPrompt {
                name: "test_prompt".into(),
                description: Some("A test prompt".into()),
                arguments: vec![],
            }]
        }

        fn get_prompt(
            &self,
            name: &str,
            _arguments: &std::collections::HashMap<String, String>,
        ) -> Result<PromptGetResult, String> {
            if name == "test_prompt" {
                Ok(PromptGetResult {
                    description: Some("A test prompt".into()),
                    messages: vec![crate::protocol::PromptMessage {
                        role: "user".into(),
                        content: crate::protocol::PromptMessageContent::Text {
                            text: "Hello".into(),
                        },
                    }],
                })
            } else {
                Err(format!("prompt not found: {name}"))
            }
        }
    }

    fn make_basic() -> McpServer<ComplianceHandler> {
        McpServer::new("compliance-test", "1.0.0", Arc::new(ComplianceHandler))
    }

    fn make_full() -> McpServer<ComplianceHandler> {
        McpServer::new("compliance-test", "1.0.0", Arc::new(ComplianceHandler))
            .with_resource_handler(Arc::new(ComplianceResourceHandler))
            .with_prompt_handler(Arc::new(CompliancePromptHandler))
    }

    fn init_params() -> serde_json::Value {
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "compliance-client", "version": "1.0"}
        })
    }

    // ─── JSON-RPC compliance ───

    #[tokio::test]
    async fn jsonrpc_version_in_response() {
        let server = make_basic();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::PING.into(),
                params: None,
            })
            .await;
        assert_eq!(resp.jsonrpc, "2.0");
    }

    #[tokio::test]
    async fn response_id_matches_request() {
        let server = make_basic();
        for id in [1, 42, 999, u64::MAX] {
            let resp = server
                .handle_request_public(JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id,
                    method: method::PING.into(),
                    params: None,
                })
                .await;
            assert_eq!(resp.id, id, "response id must match request id={id}");
        }
    }

    #[tokio::test]
    async fn error_method_not_found() {
        let server = make_basic();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: "nonexistent/method".into(),
                params: None,
            })
            .await;
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("method not found"));
    }

    #[tokio::test]
    async fn error_invalid_params_tools_call() {
        let server = make_basic();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::TOOLS_CALL.into(),
                params: Some(json!("not an object")),
            })
            .await;
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[tokio::test]
    async fn error_missing_params_tools_call() {
        let server = make_basic();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::TOOLS_CALL.into(),
                params: None,
            })
            .await;
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("missing params"));
    }

    #[tokio::test]
    async fn error_missing_params_resources_read() {
        let server = make_full();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::RESOURCES_READ.into(),
                params: None,
            })
            .await;
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[tokio::test]
    async fn error_invalid_params_resources_read() {
        let server = make_full();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::RESOURCES_READ.into(),
                params: Some(json!(42)),
            })
            .await;
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[tokio::test]
    async fn error_missing_params_prompts_get() {
        let server = make_full();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::PROMPTS_GET.into(),
                params: None,
            })
            .await;
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[tokio::test]
    async fn error_invalid_params_prompts_get() {
        let server = make_full();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::PROMPTS_GET.into(),
                params: Some(json!([])),
            })
            .await;
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[tokio::test]
    async fn error_resources_not_supported() {
        let server = make_basic(); // no resource handler
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::RESOURCES_LIST.into(),
                params: None,
            })
            .await;
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn error_prompts_not_supported() {
        let server = make_basic(); // no prompt handler
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::PROMPTS_LIST.into(),
                params: None,
            })
            .await;
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    // ─── Capability negotiation ───

    #[tokio::test]
    async fn capability_tools_always_present() {
        let server = make_basic();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::INITIALIZE.into(),
                params: Some(init_params()),
            })
            .await;
        let r: InitializeResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert!(r.capabilities.tools.is_some());
        assert!(r.capabilities.resources.is_none());
        assert!(r.capabilities.prompts.is_none());
    }

    #[tokio::test]
    async fn capability_all_present_when_handlers_set() {
        let server = make_full();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::INITIALIZE.into(),
                params: Some(init_params()),
            })
            .await;
        let r: InitializeResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert!(r.capabilities.tools.is_some());
        assert!(r.capabilities.resources.is_some());
        assert!(r.capabilities.prompts.is_some());
    }

    #[tokio::test]
    async fn protocol_version_matches_constant() {
        let server = make_basic();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::INITIALIZE.into(),
                params: Some(init_params()),
            })
            .await;
        let r: InitializeResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert_eq!(r.protocol_version, crate::protocol::MCP_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn server_info_present_and_non_empty() {
        let server = make_basic();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::INITIALIZE.into(),
                params: Some(init_params()),
            })
            .await;
        let r: InitializeResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert!(!r.server_info.name.is_empty());
        assert!(!r.server_info.version.is_empty());
    }

    // ─── Response structure ───

    #[tokio::test]
    async fn tools_list_has_tools_array() {
        let server = make_basic();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::TOOLS_LIST.into(),
                params: Some(json!({})),
            })
            .await;
        let result = resp.result.unwrap();
        assert!(result["tools"].is_array());
    }

    #[tokio::test]
    async fn resources_list_has_resources_array() {
        let server = make_full();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::RESOURCES_LIST.into(),
                params: Some(json!({})),
            })
            .await;
        let result = resp.result.unwrap();
        assert!(result["resources"].is_array());
    }

    #[tokio::test]
    async fn prompts_list_has_prompts_array() {
        let server = make_full();
        let resp = server
            .handle_request_public(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: 1,
                method: method::PROMPTS_LIST.into(),
                params: Some(json!({})),
            })
            .await;
        let result = resp.result.unwrap();
        assert!(result["prompts"].is_array());
    }

    // ─── Concurrent request handling ───

    #[tokio::test]
    async fn concurrent_requests_all_succeed() {
        let server = Arc::new(make_basic());
        let mut handles = Vec::new();
        for i in 0..10u64 {
            let s = Arc::clone(&server);
            handles.push(tokio::spawn(async move {
                s.handle_request_public(JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: i,
                    method: method::TOOLS_LIST.into(),
                    params: Some(json!({})),
                })
                .await
            }));
        }
        for (i, h) in handles.into_iter().enumerate() {
            let resp = h.await.unwrap();
            assert_eq!(resp.id, i as u64);
            assert!(resp.error.is_none());
        }
    }

    #[tokio::test]
    async fn concurrent_mixed_requests() {
        let server = Arc::new(make_full());
        let methods = vec![
            method::PING,
            method::TOOLS_LIST,
            method::RESOURCES_LIST,
            method::PROMPTS_LIST,
            method::PING,
        ];
        let mut handles = Vec::new();
        for (i, m) in methods.into_iter().enumerate() {
            let s = Arc::clone(&server);
            let method = m.to_string();
            handles.push(tokio::spawn(async move {
                s.handle_request_public(JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: i as u64,
                    method,
                    params: Some(json!({})),
                })
                .await
            }));
        }
        for h in handles {
            let resp = h.await.unwrap();
            assert!(resp.error.is_none());
        }
    }
}
