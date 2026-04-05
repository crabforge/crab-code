//! `crab serve` — run Crab Code as an MCP server.
//!
//! In stdio mode (default), reads JSON-RPC requests from stdin and writes
//! responses to stdout, suitable for use as a child process by MCP clients.
//!
//! With `--port`, starts an HTTP SSE server supporting multiple concurrent clients.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Args;

use crab_mcp::server::{McpServer, ToolRegistryHandler};
use crab_tools::builtin::create_default_registry;

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Port for HTTP SSE mode. If omitted, uses stdio transport.
    #[arg(long, short)]
    pub port: Option<u16>,

    /// Only expose tools matching these names (comma-separated).
    /// If not specified, all built-in tools are exposed.
    #[arg(long, value_delimiter = ',')]
    pub tools: Option<Vec<String>>,

    /// Working directory for tool execution.
    #[arg(long)]
    pub working_dir: Option<PathBuf>,

    /// Print server info and exit without starting.
    #[arg(long)]
    pub info: bool,
}

/// Run the MCP server.
pub async fn run(args: &ServeArgs) -> anyhow::Result<()> {
    let working_dir = args
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let registry = create_default_registry();
    let all_tools = registry.all_tools();

    // Filter tools if --tools is specified
    let tools: Vec<Arc<dyn crab_core::tool::Tool>> = match &args.tools {
        Some(filter) => all_tools
            .into_iter()
            .filter(|t| filter.iter().any(|f| f == t.name()))
            .collect(),
        None => all_tools,
    };

    let tool_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
    let tool_count = tool_names.len();
    let tool_list = tool_names.join(", ");

    if args.info {
        println!("Crab Code MCP Server");
        println!("  version:    {}", env!("CARGO_PKG_VERSION"));
        println!(
            "  transport:  {}",
            if args.port.is_some() { "sse" } else { "stdio" }
        );
        println!("  tools ({tool_count}): {tool_list}");
        return Ok(());
    }

    let handler = Arc::new(ToolRegistryHandler::new(tools, working_dir));
    let server = McpServer::new("crab-code", env!("CARGO_PKG_VERSION"), handler);

    if let Some(port) = args.port {
        // SSE mode — HTTP server with Server-Sent Events
        eprintln!(
            "crab-code MCP server v{} (SSE on port {port}, {tool_count} tools)",
            env!("CARGO_PKG_VERSION"),
        );
        let cancel = tokio_util::sync::CancellationToken::new();
        crab_mcp::run_sse(Arc::new(server), port, cancel).await?;
        return Ok(());
    }

    // Stdio mode
    eprintln!(
        "crab-code MCP server v{} (stdio, {tool_count} tools)",
        env!("CARGO_PKG_VERSION"),
    );

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    server.run(stdin, stdout).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_args_defaults() {
        let args = ServeArgs {
            port: None,
            tools: None,
            working_dir: None,
            info: false,
        };
        assert!(args.port.is_none());
        assert!(args.tools.is_none());
        assert!(args.working_dir.is_none());
        assert!(!args.info);
    }

    #[test]
    fn serve_args_with_port() {
        let args = ServeArgs {
            port: Some(8080),
            tools: None,
            working_dir: None,
            info: false,
        };
        assert_eq!(args.port, Some(8080));
    }

    #[test]
    fn serve_args_with_tool_filter() {
        let args = ServeArgs {
            port: None,
            tools: Some(vec!["bash".into(), "read".into()]),
            working_dir: None,
            info: false,
        };
        let filter = args.tools.unwrap();
        assert_eq!(filter.len(), 2);
        assert!(filter.contains(&"bash".to_string()));
    }

    #[test]
    fn serve_args_with_working_dir() {
        let args = ServeArgs {
            port: None,
            tools: None,
            working_dir: Some(PathBuf::from("/tmp/project")),
            info: false,
        };
        assert_eq!(args.working_dir.unwrap(), PathBuf::from("/tmp/project"));
    }

    #[tokio::test]
    async fn serve_info_mode() {
        let args = ServeArgs {
            port: None,
            tools: None,
            working_dir: None,
            info: true,
        };
        // --info should succeed without starting a server
        let result = run(&args).await;
        assert!(result.is_ok());
    }

    // SSE mode now works — no longer returns "not yet implemented".
    // Full SSE server tests are in crab-mcp crate (sse_server::tests).

    #[test]
    fn tool_filtering_works() {
        let registry = create_default_registry();
        let all = registry.all_tools();
        let total = all.len();

        // Filter to just "bash"
        let filter = ["bash".to_string()];
        let filtered: Vec<_> = all
            .into_iter()
            .filter(|t| filter.iter().any(|f| f == t.name()))
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name(), "bash");
        assert!(total > 1); // Sanity check there were more tools
    }
}
