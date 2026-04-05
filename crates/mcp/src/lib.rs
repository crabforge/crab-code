pub mod audit;
pub mod client;
#[cfg(test)]
mod compliance_tests;
pub mod connection_pool;
pub mod discovery;
pub mod health;
pub mod logging;
pub mod manager;
pub mod protocol;
pub mod resource;
pub mod roots;
pub mod sampling;
pub mod server;
pub mod sse_server;
pub mod tool_chain;
pub mod tool_group;
pub mod tool_sandbox;
pub mod tool_version;
pub mod transport;

pub use audit::{AuditEntry, AuditEntryBuilder, AuditOutcome, McpAuditLog};
pub use client::McpClient;
pub use connection_pool::{ConnectionPool, ConnectionState, ConnectionSummary, PoolConfig};

pub use discovery::{McpServerConfig, McpTransportConfig, connect_server, parse_mcp_servers};
pub use manager::{DiscoveredTool, McpManager};
pub use protocol::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, JsonRpcError,
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpPrompt, McpResource, McpToolDef,
    ServerCapabilities, ServerInfo, ToolCallParams, ToolCallResult,
};
pub use health::{AutoReconnect, HealthChecker, HealthCheckerConfig, HealthStatus, Heartbeat, ReconnectConfig};
pub use logging::{McpLogEntry, McpLogLevel, McpLogger};
pub use resource::ResourceCache;
pub use roots::{RootInfo, RootRegistry};
pub use sampling::{SamplingHandler, SamplingRequest, SamplingResponse, StopReason};
pub use server::{
    FileResourceHandler, McpServer, PromptHandler, ResourceHandler, SkillPromptHandler,
    ToolHandler, ToolRegistryHandler,
};
pub use sse_server::run_sse;
pub use tool_chain::{
    ChainBuilder, ChainExecutor, ChainResult, ChainStep, ToolChain, ToolChainTemplate,
};
pub use tool_group::{IndexedTool, ToolGroup, ToolIndex};
pub use tool_sandbox::{
    McpPermissionBoundary, McpToolSandbox, PermissionLevel, SandboxPolicy, SandboxVerdict,
};
pub use tool_version::{ToolVersion, ToolVersionRegistry, VersionedTool};
pub use transport::Transport;
