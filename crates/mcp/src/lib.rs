pub mod client;
pub mod discovery;
pub mod protocol;
pub mod resource;
pub mod server;
pub mod transport;

pub use client::McpClient;
pub use discovery::{McpServerConfig, McpTransportConfig};
pub use protocol::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, JsonRpcError,
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpPrompt, McpResource, McpToolDef,
    ServerCapabilities, ServerInfo, ToolCallParams, ToolCallResult,
};
pub use resource::ResourceCache;
pub use server::McpServer;
pub use transport::Transport;
