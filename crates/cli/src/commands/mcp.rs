//! MCP server mode: run crab-code as an MCP server exposing its tools.
//!
//! This subcommand starts Crab Code in MCP server mode, allowing IDE
//! extensions and other MCP clients to use Crab Code's tools over
//! stdio or SSE transport.

use std::path::PathBuf;

use clap::{Args, Subcommand};

// ── Types ────────────────────────────────────────────────────────────

/// MCP transport mode.
#[derive(Debug, Clone)]
pub enum McpTransport {
    /// JSON-RPC over stdin/stdout.
    Stdio,
    /// HTTP Server-Sent Events.
    Sse {
        /// Port to listen on.
        port: u16,
    },
}

/// Configuration for the MCP server.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Transport to use.
    pub transport: McpTransport,
    /// Tool names to expose (empty = all).
    pub tools: Vec<String>,
    /// Working directory for tool execution.
    pub working_dir: PathBuf,
}

// ── CLI Args ─────────────────────────────────────────────────────────

/// MCP subcommand arguments.
#[derive(Args, Debug)]
pub struct McpArgs {
    #[command(subcommand)]
    pub action: McpAction,
}

/// MCP subcommand actions.
#[derive(Subcommand, Debug)]
pub enum McpAction {
    /// Start an MCP server exposing Crab Code tools.
    Serve {
        /// Port for HTTP SSE mode. If omitted, uses stdio transport.
        #[arg(long, short)]
        port: Option<u16>,

        /// Only expose tools matching these names (comma-separated).
        #[arg(long, value_delimiter = ',')]
        tools: Option<Vec<String>>,

        /// Working directory for tool execution.
        #[arg(long)]
        working_dir: Option<PathBuf>,
    },
    /// List available MCP tools.
    List,
    /// Show MCP server status and diagnostics.
    Status,
}

// ── Entry point ──────────────────────────────────────────────────────

/// Run the MCP subcommand.
pub async fn run(args: &McpArgs) -> anyhow::Result<()> {
    match &args.action {
        McpAction::Serve {
            port,
            tools,
            working_dir,
        } => {
            let config = McpServerConfig {
                transport: match port {
                    Some(p) => McpTransport::Sse { port: *p },
                    None => McpTransport::Stdio,
                },
                tools: tools.clone().unwrap_or_default(),
                working_dir: working_dir.clone().unwrap_or_else(|| {
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                }),
            };
            run_mcp_server(config).await
        }
        McpAction::List => list_mcp_tools().await,
        McpAction::Status => show_mcp_status().await,
    }
}

/// Start the MCP server with the given configuration.
pub async fn run_mcp_server(config: McpServerConfig) -> anyhow::Result<()> {
    let _ = config;
    todo!("run_mcp_server — initialize tool registry and start MCP transport")
}

/// List all tools that would be exposed via MCP.
async fn list_mcp_tools() -> anyhow::Result<()> {
    todo!("list_mcp_tools — enumerate available tools with names and descriptions")
}

/// Show MCP server status and diagnostics.
async fn show_mcp_status() -> anyhow::Result<()> {
    todo!("show_mcp_status — report running server info, connected clients")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_transport_variants() {
        let stdio = McpTransport::Stdio;
        assert!(matches!(stdio, McpTransport::Stdio));

        let sse = McpTransport::Sse { port: 8080 };
        assert!(matches!(sse, McpTransport::Sse { port: 8080 }));
    }

    #[test]
    fn mcp_server_config_construction() {
        let config = McpServerConfig {
            transport: McpTransport::Stdio,
            tools: vec!["Bash".into(), "Read".into()],
            working_dir: PathBuf::from("/tmp"),
        };
        assert_eq!(config.tools.len(), 2);
        assert!(matches!(config.transport, McpTransport::Stdio));
    }
}
